use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const SCHEMA_VERSION: u16 = 1;

/// Open, validated identity. Provider and risk namespaces are intentionally extensible.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Symbol(String);

impl Symbol {
    pub fn new(value: impl Into<String>) -> Result<Self, EstateError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 96
            && !value.contains("..")
            && !value.starts_with(['/', '.', '-'])
            && !value.ends_with(['.', '-'])
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-' | '_' | '.'));
        valid
            .then_some(Self(value))
            .ok_or(EstateError::InvalidSymbol)
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderCard {
    pub version: u16,
    pub provider: Symbol,
    pub capabilities: Vec<Symbol>,
    pub max_targets: u32,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub version: u16,
    pub id: Symbol,
    pub labels: BTreeMap<Symbol, Symbol>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SanitizedInventory {
    pub version: u16,
    pub revision: u64,
    pub targets: Vec<Target>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFingerprint {
    pub version: u16,
    pub digest: [u8; 32],
}
impl ProjectFingerprint {
    #[must_use]
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            version: SCHEMA_VERSION,
            digest: Sha256::digest(bytes).into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum OperationMode {
    Inspect,
    Change,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub version: u16,
    pub exposure: Symbol,
    pub target: Symbol,
    pub parameters: BTreeMap<Symbol, Value>,
    pub mode: OperationMode,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Integer(i64),
    Text(String),
    Symbol(Symbol),
    List(Vec<Value>),
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u16,
    pub id: Symbol,
    pub operation: Operation,
    pub inventory_revision: u64,
    pub project: ProjectFingerprint,
    pub risk: Symbol,
    pub preview: Option<Preview>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preview {
    pub changed_targets: u32,
    pub notices: Vec<Symbol>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Approval {
    pub version: u16,
    pub plan: Symbol,
    pub project: ProjectFingerprint,
    pub granted: bool,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub version: u16,
    pub id: Symbol,
    pub plan: Symbol,
    pub dispatched_at: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    Accepted,
    Progress,
    Changed,
    Unchanged,
    Cancelled,
    Malformed,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub version: u16,
    pub sequence: u32,
    pub at: u64,
    pub kind: EventKind,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    pub version: u16,
    pub run: Symbol,
    pub passed: bool,
    pub checks: Vec<Symbol>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reconciliation {
    pub version: u16,
    pub run: Symbol,
    pub state: RunState,
    pub observed_at: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RunState {
    NotDispatched,
    Running,
    Changed,
    Unchanged,
    Cancelled,
    Failed,
    UnknownAfterDispatch,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lease {
    pub version: u16,
    pub resource: Symbol,
    pub holder: Symbol,
    pub expires_at: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quarantine {
    pub version: u16,
    pub target: Symbol,
    pub reason: Symbol,
    pub until: Option<u64>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub version: u16,
    pub run: Option<Symbol>,
    pub state: RunState,
    pub verification: Option<Verification>,
    pub reconciliation: Option<Reconciliation>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error, Serialize, Deserialize)]
pub enum EstateError {
    #[error("unsupported: {0:?}")]
    Unsupported(Symbol),
    #[error("invalid symbol")]
    InvalidSymbol,
    #[error("bound exceeded: {0:?}")]
    BoundExceeded(Symbol),
    #[error("stale plan")]
    StalePlan,
    #[error("target disappeared")]
    TargetDisappeared,
    #[error("verification failed")]
    VerificationFailed,
    #[error("malformed provider event")]
    MalformedEvent,
    #[error("cancelled")]
    Cancelled,
    #[error("not dispatched")]
    NotDispatched,
    #[error("outcome unknown after dispatch")]
    UnknownAfterDispatch,
}

/// A provider returns only portable, bounded values or typed refusal.
pub trait EstateProvider {
    fn discover(&mut self) -> Result<(ProviderCard, SanitizedInventory), EstateError>;
    fn plan(
        &mut self,
        operation: Operation,
        project: ProjectFingerprint,
    ) -> Result<Plan, EstateError>;
    fn preview(&mut self, plan: &Plan) -> Result<Preview, EstateError>;
    fn perform(&mut self, plan: &Plan, approval: &Approval) -> Result<Run, EstateError>;
    fn events(&mut self, run: &Run, limit: u32) -> Result<Vec<Event>, EstateError>;
    fn verify(&mut self, run: &Run) -> Result<Verification, EstateError>;
    fn reconcile(&mut self, run: &Run) -> Result<Reconciliation, EstateError>;
    fn cancel(&mut self, run: &Run) -> Result<Outcome, EstateError>;
}

/// Shared provider conformance scenario, reusable by physical and modeled sites.
pub fn conformance_smoke(
    provider: &mut impl EstateProvider,
    operation: Operation,
    project: ProjectFingerprint,
) -> Result<Outcome, EstateError> {
    let (_, inventory) = provider.discover()?;
    if inventory.targets.len() > 1024 {
        return Err(EstateError::BoundExceeded(Symbol::new("inventory")?));
    }
    let plan = provider.plan(operation, project.clone())?;
    let _ = provider.preview(&plan).or_else(|e| match e {
        EstateError::Unsupported(_) => Ok(Preview {
            changed_targets: 0,
            notices: vec![],
        }),
        other => Err(other),
    })?;
    let approval = Approval {
        version: SCHEMA_VERSION,
        plan: plan.id.clone(),
        project,
        granted: true,
    };
    let run = provider.perform(&plan, &approval)?;
    let events = provider.events(&run, 256)?;
    if events
        .iter()
        .any(|event| matches!(event.kind, EventKind::Malformed))
    {
        return Err(EstateError::MalformedEvent);
    }
    let verification = provider.verify(&run)?;
    let reconciliation = provider.reconcile(&run)?;
    Ok(Outcome {
        version: SCHEMA_VERSION,
        run: Some(run.id),
        state: reconciliation.state.clone(),
        verification: Some(verification),
        reconciliation: Some(reconciliation),
    })
}
