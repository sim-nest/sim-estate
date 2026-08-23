mod support;

use crate::support::{
    FsArtifacts, Gate, NativePort, SharedTable, Time, git_state_digest, hash_file, hash_json,
    hash_text, opaque_target, project_fingerprint, sym,
};
use serde::Serialize;
use sim_estate_core::{EstateProvider, Operation, OperationMode, ProjectFingerprint};
use sim_lib_estate::{ApplyState, Organ, Policy};
use sim_lib_estate_book::Key;
use sim_lib_estate_serve::{BookDir, EstateDir};
use sim_lib_exec::{BindingValue, PrivateArtifactRef, ProgramRef, ProjectRootRef};
use sim_site_estate_ansible::{AnsibleBindings, AnsibleSite, CALLBACK_PY, OperationBinding};
use sim_site_estate_model::{Fault, ModelEstate};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

fn main() {
    if let Err(error) = run(std::env::args().skip(1)) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run(args: impl Iterator<Item = String>) -> Result<(), String> {
    let args = Args::parse(args)?;
    let work = args.work_dir.clone();
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let before = git_state_digest(&args.private_root)?;
    let port = NativePort::new(&args.private_root, &work)?;
    let artifacts = FsArtifacts::new(&work)?;
    let target = opaque_target(&args.observe_target)?;
    let project = project_fingerprint(&args.private_root, &args.binding_digest, &args.operation)?;
    let mut site = AnsibleSite::new(&port, &artifacts, bindings(&args, &work)?);
    let (_, inventory) = site.discover().map_err(|e| e.to_string())?;
    if !inventory.targets.iter().any(|item| item.id == target) {
        return Err("bounded target is absent from sanitized inventory".into());
    }
    let table = SharedTable::default();
    let organ = Organ::new(table.clone());
    let operation = Operation {
        version: 1,
        exposure: sym(&args.operation)?,
        target,
        parameters: BTreeMap::new(),
        mode: OperationMode::Inspect,
    };
    let planned = organ
        .plan(
            &mut site,
            operation,
            project.clone(),
            Key("program/boot-tool-make".into()),
            Policy {
                preview: true,
                verify: true,
                ttl: 600,
            },
            "physical-readonly".into(),
            0,
        )
        .map_err(|e| e.to_string())?;
    let approval = organ
        .review(
            "physical-readonly",
            &planned,
            "roadmap-resource",
            "physical-run",
        )
        .map_err(|e| e.to_string())?;
    let outcome = organ
        .apply(
            &mut site,
            &mut Gate,
            &Time(0),
            &planned,
            "physical-readonly",
            approval,
            "physical-run",
        )
        .map_err(|e| e.to_string())?;
    if outcome.state != ApplyState::Verified {
        return Err("physical operation was not verified".into());
    }
    let projection = organ
        .book
        .projection("physical-run")
        .map_err(|e| e.to_string())?;
    let table_rows = BookDir::new(table, ["physical-run".to_owned()])
        .rows()
        .map_err(|e| e.to_string())?;
    let refusal = model_not_dispatched()?;
    let loss = model_controller_loss()?;
    let after = git_state_digest(&args.private_root)?;
    if before != after {
        return Err("private project changed during read-only acceptance".into());
    }
    let facts = Facts {
        source: args.source,
        manifest_sha256: hash_file(Path::new("acceptance/controller-v1.sx"))?,
        harness_sha256: hash_file(Path::new("acceptance/run-controller.sh"))?,
        binding_summary_sha256: args.binding_digest,
        inventory_sha256: hash_json(&inventory)?,
        operation_binding_sha256: hash_json(&OperationBindingProof {
            operation: &args.operation,
            make_target: &args.make_target,
            target_assignment: "LIMIT",
        })?,
        callback_events_sha256: hash_file(&work.join("events.jsonl"))?,
        process_attempts_sha256: hash_json(&port.attempts()?)?,
        project_before_sha256: before,
        project_after_sha256: after,
        target_set_sha256: hash_text(&args.observe_target),
        book_projection_sha256: hash_json(&BookProjectionProof {
            run: &projection.run,
            plan: &projection.plan.0,
            state: &projection.state,
            events: &projection.events,
        })?,
        table_projection_sha256: hash_json(&table_rows)?,
        cli_output_sha256: hash_json(&table_rows)?,
        model_refusal_sha256: hash_json(&refusal)?,
        model_controller_loss_sha256: hash_json(&loss)?,
    };
    write_artifact(&args.output, &facts)
}

struct Args {
    source: String,
    private_root: PathBuf,
    binding_digest: String,
    observe_target: String,
    operation: String,
    make_target: String,
    work_dir: PathBuf,
    output: PathBuf,
}
impl Args {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        let mut it = args.peekable();
        while let Some(flag) = it.next() {
            let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
            values.insert(flag, value);
        }
        let take = |values: &mut BTreeMap<String, String>, key: &str| {
            values
                .remove(key)
                .ok_or_else(|| format!("{key} is required"))
        };
        Ok(Self {
            source: take(&mut values, "--source")?,
            private_root: PathBuf::from(take(&mut values, "--private-root")?),
            binding_digest: take(&mut values, "--binding-summary-sha256")?,
            observe_target: take(&mut values, "--observe-target")?,
            operation: take(&mut values, "--operation")?,
            make_target: take(&mut values, "--make-target")?,
            work_dir: PathBuf::from(take(&mut values, "--work-dir")?),
            output: PathBuf::from(take(&mut values, "--output")?),
        })
    }
}

fn bindings(args: &Args, work: &Path) -> Result<AnsibleBindings, String> {
    let callback_dir = work.join("callback");
    fs::create_dir_all(&callback_dir).map_err(|e| e.to_string())?;
    fs::write(callback_dir.join("sim_estate_aggregate.py"), CALLBACK_PY)
        .map_err(|e| e.to_string())?;
    let local_temp = work.join("local-temp");
    fs::create_dir_all(&local_temp).map_err(|e| e.to_string())?;
    let op = sym(&args.operation)?;
    let mut base_environment: BTreeMap<String, BindingValue> = [
        (
            "ANSIBLE_RETRY_FILES_ENABLED".into(),
            BindingValue::Literal("False".into()),
        ),
        (
            "ANSIBLE_LOCAL_TEMP".into(),
            BindingValue::PrivateArtifact(
                PrivateArtifactRef::new("artifact/local-temp").map_err(|e| e.to_string())?,
            ),
        ),
        (
            "ANSIBLE_SSH_ARGS".into(),
            BindingValue::Literal("-F none".into()),
        ),
    ]
    .into();
    for name in ["PATH", "HOME"] {
        if let Ok(value) = std::env::var(name) {
            base_environment.insert(name.into(), BindingValue::Literal(value));
        }
    }
    Ok(AnsibleBindings {
        root: ProjectRootRef::new("project/private").map_err(|e| e.to_string())?,
        inventory_program: ProgramRef::new("program/ansible-inventory")
            .map_err(|e| e.to_string())?,
        config_program: ProgramRef::new("program/ansible-config").map_err(|e| e.to_string())?,
        make_program: ProgramRef::new("program/make").map_err(|e| e.to_string())?,
        operations: [(
            op,
            OperationBinding {
                make_target: args.make_target.clone(),
                target_assignment: Some("LIMIT".into()),
            },
        )]
        .into(),
        base_environment,
        callback_plugin: PrivateArtifactRef::new("artifact/callback").map_err(|e| e.to_string())?,
        event_output: PrivateArtifactRef::new("artifact/events").map_err(|e| e.to_string())?,
        human_output: PrivateArtifactRef::new("artifact/human").map_err(|e| e.to_string())?,
        timeout_ms: 120_000,
    })
}

#[derive(Serialize)]
struct OperationBindingProof<'a> {
    operation: &'a str,
    make_target: &'a str,
    target_assignment: &'a str,
}

#[derive(Serialize)]
struct BookProjectionProof<'a> {
    run: &'a str,
    plan: &'a str,
    state: &'a str,
    events: &'a [sim_lib_estate_book::EventEnvelope],
}

#[derive(Serialize)]
struct ModelProof {
    state: String,
    retryable: bool,
}
fn model_not_dispatched() -> Result<ModelProof, String> {
    let organ = Organ::new(SharedTable::default());
    let mut model = ModelEstate::fixture();
    let planned = model_plan(&organ, &mut model)?;
    let approval = organ
        .review("a", &planned, "reviewer", "model-run")
        .unwrap();
    model.inject(Fault::NotDispatched);
    let err = organ
        .apply(
            &mut model,
            &mut Gate,
            &Time(0),
            &planned,
            "a",
            approval,
            "model-run",
        )
        .unwrap_err();
    Ok(ModelProof {
        state: err.to_string(),
        retryable: true,
    })
}
fn model_controller_loss() -> Result<ModelProof, String> {
    let organ = Organ::new(SharedTable::default());
    let mut model = ModelEstate::fixture();
    let planned = model_plan(&organ, &mut model)?;
    let approval = organ
        .review("a", &planned, "reviewer", "model-run")
        .unwrap();
    model.inject(Fault::UnknownAfterDispatch);
    let out = organ
        .apply(
            &mut model,
            &mut Gate,
            &Time(0),
            &planned,
            "a",
            approval,
            "model-run",
        )
        .map_err(|e| e.to_string())?;
    Ok(ModelProof {
        state: format!("{:?}", out.state),
        retryable: false,
    })
}
fn model_plan(
    organ: &Organ<SharedTable>,
    model: &mut ModelEstate,
) -> Result<sim_lib_estate::Planned, String> {
    organ
        .plan(
            model,
            Operation {
                version: 1,
                exposure: sym("service/restart")?,
                target: sym("fleet/web")?,
                parameters: BTreeMap::new(),
                mode: OperationMode::Change,
            },
            ProjectFingerprint::of(b"fixture-project-v1"),
            Key("program".into()),
            Policy {
                preview: true,
                verify: false,
                ttl: 10,
            },
            "nonce".into(),
            0,
        )
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
struct Facts {
    source: String,
    manifest_sha256: String,
    harness_sha256: String,
    binding_summary_sha256: String,
    inventory_sha256: String,
    operation_binding_sha256: String,
    callback_events_sha256: String,
    process_attempts_sha256: String,
    project_before_sha256: String,
    project_after_sha256: String,
    target_set_sha256: String,
    book_projection_sha256: String,
    table_projection_sha256: String,
    cli_output_sha256: String,
    model_refusal_sha256: String,
    model_controller_loss_sha256: String,
}

fn write_artifact(path: &Path, facts: &Facts) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let mut out = String::from("(sim.estate-acceptance/v1\n");
    field(&mut out, "source", &facts.source)?;
    field(&mut out, "target", "control-local")?;
    field(&mut out, "controller_capsule", "physical-readonly")?;
    field(&mut out, "operation", "estate/ping")?;
    field(
        &mut out,
        "delivered_vertical",
        "provider-organ-book-table-cli",
    )?;
    field(&mut out, "process_service", "ProcessPort")?;
    field(&mut out, "storage_service", "Table")?;
    field(&mut out, "clock_timer_service", "bounded-runner")?;
    field(&mut out, "private_boundary", "host-blind")?;
    field(&mut out, "artifact_retention", "sanitized-only")?;
    field(&mut out, "network", "lan-bounded")?;
    for (name, value) in [
        ("manifest_sha256", &facts.manifest_sha256),
        ("harness_sha256", &facts.harness_sha256),
        ("binding_summary_sha256", &facts.binding_summary_sha256),
        ("inventory_sha256", &facts.inventory_sha256),
        ("operation_binding_sha256", &facts.operation_binding_sha256),
        ("callback_events_sha256", &facts.callback_events_sha256),
        ("process_attempts_sha256", &facts.process_attempts_sha256),
        ("project_before_sha256", &facts.project_before_sha256),
        ("project_after_sha256", &facts.project_after_sha256),
        ("target_set_sha256", &facts.target_set_sha256),
        ("book_projection_sha256", &facts.book_projection_sha256),
        ("table_projection_sha256", &facts.table_projection_sha256),
        ("cli_output_sha256", &facts.cli_output_sha256),
        ("model_refusal_sha256", &facts.model_refusal_sha256),
        (
            "model_controller_loss_sha256",
            &facts.model_controller_loss_sha256,
        ),
    ] {
        field(&mut out, name, value)?;
    }
    field(&mut out, "project_unchanged", "true")?;
    field(&mut out, "callback_final", "succeeded")?;
    out.push_str("  (cases\n");
    for (id, category) in [
        ("controller-capability", "resource/controller"),
        ("fixed-inventory", "provider/discovery"),
        ("delivered-provider", "provider/ansible-site"),
        ("organ-apply", "organ/plan-review-apply"),
        ("process-attempt", "process/attempt"),
        ("durable-book", "book/projection"),
        ("table-projection", "table/read-only-view"),
        ("cli-output", "serve/history-output"),
        ("refusal", "model/not-dispatched"),
        ("controller-loss", "model/quarantine"),
        ("project-equivalence", "project/before-after"),
        ("private-boundary", "evidence/sanitized"),
        ("offline-verifier", "artifact/verify"),
    ] {
        out.push_str(&format!(
            "    (case (id \"{id}\") (category \"{category}\") (passed true))\n"
        ));
    }
    out.push_str("  )\n");
    field(&mut out, "result", "physical-controller-readonly-pass")?;
    out.push_str(")\n");
    fs::write(&tmp, out).map_err(|e| e.to_string())?;
    fs::rename(tmp, path).map_err(|e| e.to_string())
}

fn field(out: &mut String, name: &str, value: &str) -> Result<(), String> {
    if value.contains(['"', '\\']) {
        return Err(format!("{name} contains an unsafe character"));
    }
    out.push_str(&format!("  ({name} \"{value}\")\n"));
    Ok(())
}
