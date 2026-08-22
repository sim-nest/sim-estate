#![forbid(unsafe_code)]
//! Ansible as a sealed decoration over the canonical process port.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use sim_estate_core::{
    Approval, EstateError, EstateProvider, Event, Operation, Outcome, Plan, Preview,
    ProjectFingerprint, ProviderCard, Reconciliation, Run, RunState, SCHEMA_VERSION,
    SanitizedInventory, Symbol, Target, Verification,
};
use sim_lib_exec::{
    ArgAtom, BindingValue, PrivateArtifactRef, ProcessAttempt, ProcessBudget, ProcessCancellation,
    ProcessPort, ProcessRequest, ProgramRef, ProjectRootRef, SealedBindings,
};
use std::collections::{BTreeMap, BTreeSet};

pub const CALLBACK_PY: &str = include_str!("../assets/sim_estate_callback.py");
const MAX_INVENTORY_BYTES: usize = 1_048_576;
mod protocol;
pub use protocol::{DecodedEvents, decode_events};

pub trait PrivateArtifacts: Send + Sync {
    fn read(&self, artifact: &PrivateArtifactRef) -> Result<Vec<u8>, EstateError>;
}

#[derive(Clone, Debug)]
pub struct AnsibleBindings {
    pub root: ProjectRootRef,
    pub inventory_program: ProgramRef,
    pub config_program: ProgramRef,
    pub make_program: ProgramRef,
    pub operations: BTreeMap<Symbol, OperationBinding>,
    pub base_environment: BTreeMap<String, BindingValue>,
    pub callback_plugin: PrivateArtifactRef,
    pub event_output: PrivateArtifactRef,
    pub human_output: PrivateArtifactRef,
    pub timeout_ms: u64,
}
#[derive(Clone, Debug)]
pub struct OperationBinding {
    pub make_target: String,
    pub target_assignment: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InventoryGroup {
    #[serde(default)]
    hosts: Vec<String>,
    #[serde(default)]
    children: Vec<String>,
}

pub fn decode_inventory(input: &[u8]) -> Result<SanitizedInventory, EstateError> {
    decode_inventory_with_targets(input).map(|(inventory, _)| inventory)
}

fn decode_inventory_with_targets(
    input: &[u8],
) -> Result<(SanitizedInventory, BTreeMap<Symbol, String>), EstateError> {
    if input.len() > MAX_INVENTORY_BYTES {
        return Err(bound("inventory"));
    }
    let root: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(input).map_err(|_| EstateError::MalformedEvent)?;
    let mut targets = BTreeSet::new();
    let mut raw_targets = BTreeMap::new();
    let mut memberships: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (group, value) in root {
        if group == "_meta" {
            continue;
        }
        let group_id = opaque("group", &group);
        let parsed: InventoryGroup =
            serde_json::from_value(value).map_err(|_| EstateError::MalformedEvent)?;
        for host in parsed.hosts {
            let host_id = opaque("host", &host);
            targets.insert(host_id.clone());
            raw_targets.insert(Symbol::new(host_id.clone())?, host);
            memberships
                .entry(host_id)
                .or_default()
                .insert(group_id.clone());
        }
        for child in parsed.children {
            let child_id = opaque("group", &child);
            memberships
                .entry(child_id)
                .or_default()
                .insert(group_id.clone());
        }
    }
    if targets.len() > 1024 {
        return Err(bound("inventory"));
    }
    let targets = targets
        .into_iter()
        .map(|id| {
            let labels = memberships
                .remove(&id)
                .unwrap_or_default()
                .into_iter()
                .enumerate()
                .map(|(n, group)| {
                    Ok((Symbol::new(format!("membership/{n}"))?, Symbol::new(group)?))
                })
                .collect::<Result<BTreeMap<_, _>, EstateError>>()?;
            Ok(Target {
                version: SCHEMA_VERSION,
                id: Symbol::new(id)?,
                labels,
            })
        })
        .collect::<Result<Vec<_>, EstateError>>()?;
    let digest =
        Sha256::digest(serde_json::to_vec(&targets).map_err(|_| EstateError::MalformedEvent)?);
    let mut revision_bytes = [0_u8; 8];
    revision_bytes.copy_from_slice(&digest[..8]);
    Ok((
        SanitizedInventory {
            version: SCHEMA_VERSION,
            revision: u64::from_be_bytes(revision_bytes),
            targets,
        },
        raw_targets,
    ))
}

fn opaque(namespace: &str, value: &str) -> String {
    format!(
        "{namespace}/{}",
        hex(&Sha256::digest(value.as_bytes())[..16])
    )
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut output, value| {
        write!(output, "{value:02x}").expect("writing to String cannot fail");
        output
    })
}
pub(crate) fn bound(name: &str) -> EstateError {
    EstateError::BoundExceeded(Symbol::new(name).expect("static symbol"))
}

pub struct AnsibleSite<'a> {
    port: &'a dyn ProcessPort,
    artifacts: &'a dyn PrivateArtifacts,
    bindings: AnsibleBindings,
    inventory: Option<SanitizedInventory>,
    target_bindings: BTreeMap<Symbol, String>,
    plans: BTreeMap<Symbol, Plan>,
    runs: BTreeMap<Symbol, Symbol>,
    dispatches: u64,
}

impl<'a> AnsibleSite<'a> {
    pub fn new(
        port: &'a dyn ProcessPort,
        artifacts: &'a dyn PrivateArtifacts,
        bindings: AnsibleBindings,
    ) -> Self {
        Self {
            port,
            artifacts,
            bindings,
            inventory: None,
            target_bindings: BTreeMap::new(),
            plans: BTreeMap::new(),
            runs: BTreeMap::new(),
            dispatches: 0,
        }
    }
    #[must_use]
    pub fn dispatch_count(&self) -> u64 {
        self.dispatches
    }
    fn artifact_events(&self, run: &Run, complete: bool) -> Result<DecodedEvents, EstateError> {
        let plan = self.runs.get(&run.id).ok_or(EstateError::NotDispatched)?;
        decode_events(
            &self.artifacts.read(&self.bindings.event_output)?,
            &run.id,
            plan,
            complete,
        )
    }
    fn callback_environment(
        &self,
        run: &Symbol,
        plan: &Symbol,
    ) -> Result<SealedBindings, EstateError> {
        let mut entries = self.bindings.base_environment.clone();
        entries.extend([
            (
                "ANSIBLE_CALLBACK_PLUGINS".into(),
                BindingValue::PrivateArtifact(self.bindings.callback_plugin.clone()),
            ),
            (
                "ANSIBLE_CALLBACKS_ENABLED".into(),
                BindingValue::Literal("sim_estate_aggregate".into()),
            ),
            (
                "ANSIBLE_LOAD_CALLBACK_PLUGINS".into(),
                BindingValue::Literal("1".into()),
            ),
            (
                "ANSIBLE_SHOW_PER_HOST_START".into(),
                BindingValue::Literal("0".into()),
            ),
            (
                "SIM_ESTATE_RUN".into(),
                BindingValue::Literal(run.as_str().into()),
            ),
            (
                "SIM_ESTATE_PLAN".into(),
                BindingValue::Literal(plan.as_str().into()),
            ),
            (
                "SIM_ESTATE_EVENTS".into(),
                BindingValue::PrivateArtifact(self.bindings.event_output.clone()),
            ),
        ]);
        SealedBindings::try_from_entries(entries).map_err(|_| EstateError::MalformedEvent)
    }

    fn private_artifacts(&self) -> Vec<PrivateArtifactRef> {
        let mut values = vec![
            self.bindings.callback_plugin.clone(),
            self.bindings.event_output.clone(),
            self.bindings.human_output.clone(),
        ];
        values.extend(
            self.bindings
                .base_environment
                .values()
                .filter_map(|value| match value {
                    BindingValue::PrivateArtifact(v) => Some(v.clone()),
                    _ => None,
                }),
        );
        values
    }

    fn operation_request(
        &self,
        argv: Vec<String>,
        run: &Symbol,
        plan: &Symbol,
    ) -> Result<ProcessRequest, EstateError> {
        Ok(ProcessRequest {
            program: self.bindings.make_program.clone(),
            argv: argv
                .into_iter()
                .map(|value| ArgAtom::new(value).map_err(|_| EstateError::InvalidSymbol))
                .collect::<Result<_, _>>()?,
            root: self.bindings.root.clone(),
            environment: self.callback_environment(run, plan)?,
            private_artifacts: self.private_artifacts(),
            budget: ProcessBudget {
                timeout_ms: self.bindings.timeout_ms,
                max_output_bytes: 65_536,
                stdin: None,
            },
        })
    }

    fn verify_effective_config(&mut self, run: &Symbol, plan: &Symbol) -> Result<(), EstateError> {
        let operation = self.operation_request(vec!["perform".into()], run, plan)?;
        let request = ProcessRequest {
            program: self.bindings.config_program.clone(),
            argv: ["dump", "--only-changed", "--format", "json"]
                .into_iter()
                .map(|value| ArgAtom::new(value).expect("fixed atom"))
                .collect(),
            root: operation.root,
            environment: operation.environment,
            private_artifacts: operation.private_artifacts,
            budget: ProcessBudget {
                timeout_ms: self.bindings.timeout_ms,
                max_output_bytes: 65_536,
                stdin: None,
            },
        };
        self.dispatches += 1;
        let ProcessAttempt::Completed { receipt } =
            self.port.run(&request, &ProcessCancellation::default())
        else {
            return Err(EstateError::NotDispatched);
        };
        if receipt.result.truncated || receipt.result.exit_code != 0 {
            return Err(EstateError::MalformedEvent);
        }
        let rows: Vec<ConfigRow> = serde_json::from_str(&receipt.result.stdout)
            .map_err(|_| EstateError::MalformedEvent)?;
        let required = [
            "CALLBACKS_ENABLED",
            "DEFAULT_LOAD_CALLBACK_PLUGINS",
            "SHOW_PER_HOST_START",
        ];
        if required.iter().any(|name| {
            !rows
                .iter()
                .any(|row| row.name == *name && row.source == "env")
        }) {
            return Err(EstateError::MalformedEvent);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigRow {
    name: String,
    source: String,
}

impl EstateProvider for AnsibleSite<'_> {
    fn discover(&mut self) -> Result<(ProviderCard, SanitizedInventory), EstateError> {
        let request = ProcessRequest {
            program: self.bindings.inventory_program.clone(),
            argv: ["--list"]
                .into_iter()
                .map(|v| ArgAtom::new(v).expect("literal"))
                .collect(),
            root: self.bindings.root.clone(),
            environment: SealedBindings::try_from_entries(self.bindings.base_environment.clone())
                .map_err(|_| EstateError::MalformedEvent)?,
            private_artifacts: self.private_artifacts(),
            budget: ProcessBudget {
                timeout_ms: self.bindings.timeout_ms,
                max_output_bytes: MAX_INVENTORY_BYTES,
                stdin: None,
            },
        };
        self.dispatches += 1;
        let ProcessAttempt::Completed { receipt } =
            self.port.run(&request, &ProcessCancellation::default())
        else {
            return Err(EstateError::NotDispatched);
        };
        if receipt.result.truncated || receipt.result.exit_code != 0 {
            return Err(EstateError::MalformedEvent);
        }
        let (inventory, targets) = decode_inventory_with_targets(receipt.result.stdout.as_bytes())?;
        self.inventory = Some(inventory.clone());
        self.target_bindings = targets;
        Ok((
            ProviderCard {
                version: 1,
                provider: Symbol::new("provider/ansible")?,
                capabilities: vec![Symbol::new("observe")?, Symbol::new("change")?],
                max_targets: 1024,
            },
            inventory,
        ))
    }
    fn plan(
        &mut self,
        operation: Operation,
        project: ProjectFingerprint,
    ) -> Result<Plan, EstateError> {
        let inventory = self
            .inventory
            .as_ref()
            .ok_or(EstateError::TargetDisappeared)?;
        if !inventory
            .targets
            .iter()
            .any(|target| target.id == operation.target)
        {
            return Err(EstateError::TargetDisappeared);
        }
        let operation_hash = hex(&Sha256::digest(
            serde_json::to_vec(&operation).map_err(|_| EstateError::MalformedEvent)?,
        ));
        let id = Symbol::new(format!("plan/{}", &operation_hash[..24]))?;
        let plan = Plan {
            version: 1,
            id: id.clone(),
            operation,
            inventory_revision: inventory.revision,
            project,
            risk: Symbol::new("risk/medium")?,
            preview: None,
        };
        self.plans.insert(id, plan.clone());
        Ok(plan)
    }
    fn preview(&mut self, plan: &Plan) -> Result<Preview, EstateError> {
        if !plan.operation.exposure.as_str().ends_with("/preview") {
            return Err(EstateError::Unsupported(Symbol::new("preview")?));
        }
        Ok(Preview {
            changed_targets: 0,
            notices: vec![],
        })
    }
    fn perform(&mut self, plan: &Plan, approval: &Approval) -> Result<Run, EstateError> {
        if !approval.granted || approval.plan != plan.id || approval.project != plan.project {
            return Err(EstateError::StalePlan);
        }
        let run_id = Symbol::new(format!(
            "run/{}",
            &hex(&Sha256::digest(plan.id.as_str()))[..24]
        ))?;
        self.verify_effective_config(&run_id, &plan.id)?;
        let binding = self
            .bindings
            .operations
            .get(&plan.operation.exposure)
            .ok_or_else(|| EstateError::Unsupported(plan.operation.exposure.clone()))?;
        let mut argv = vec![binding.make_target.clone()];
        if let Some(name) = &binding.target_assignment {
            let raw = self
                .target_bindings
                .get(&plan.operation.target)
                .ok_or(EstateError::TargetDisappeared)?;
            argv.push(format!("{name}={raw}"));
        }
        let request = self.operation_request(argv, &run_id, &plan.id)?;
        self.dispatches += 1;
        let attempt = self.port.run(&request, &ProcessCancellation::default());
        if matches!(attempt, ProcessAttempt::NotDispatched { .. }) {
            return Err(EstateError::NotDispatched);
        }
        self.runs.insert(run_id.clone(), plan.id.clone());
        Ok(Run {
            version: 1,
            id: run_id,
            plan: plan.id.clone(),
            dispatched_at: self.dispatches,
        })
    }
    fn events(&mut self, run: &Run, limit: u32) -> Result<Vec<Event>, EstateError> {
        let mut decoded = self.artifact_events(run, false)?.events;
        decoded.truncate(limit as usize);
        Ok(decoded)
    }
    fn verify(&mut self, run: &Run) -> Result<Verification, EstateError> {
        let decoded = self.artifact_events(run, true)?;
        Ok(Verification {
            version: 1,
            run: run.id.clone(),
            passed: !matches!(decoded.state, RunState::Failed),
            checks: vec![Symbol::new("callback/hash-chain")?],
        })
    }
    fn reconcile(&mut self, run: &Run) -> Result<Reconciliation, EstateError> {
        let decoded = self.artifact_events(run, true)?;
        Ok(Reconciliation {
            version: 1,
            run: run.id.clone(),
            state: decoded.state,
            observed_at: self.dispatches,
        })
    }
    fn cancel(&mut self, run: &Run) -> Result<Outcome, EstateError> {
        Ok(Outcome {
            version: 1,
            run: Some(run.id.clone()),
            state: RunState::Cancelled,
            verification: None,
            reconciliation: None,
        })
    }
}

#[cfg(test)]
mod tests;
