use super::*;
use sim_lib_exec::{ProcResult, ProcessReceipt};
use std::sync::Mutex;

struct Artifacts(Mutex<Vec<u8>>);
impl PrivateArtifacts for Artifacts {
    fn read(&self, _: &PrivateArtifactRef) -> Result<Vec<u8>, EstateError> {
        Ok(self.0.lock().unwrap().clone())
    }
}
struct Port {
    outcomes: Mutex<Vec<ProcessAttempt>>,
    requests: Mutex<Vec<ProcessRequest>>,
}
impl ProcessPort for Port {
    fn run(&self, request: &ProcessRequest, _: &ProcessCancellation) -> ProcessAttempt {
        self.requests.lock().unwrap().push(request.clone());
        self.outcomes.lock().unwrap().remove(0)
    }
}
fn completed(stdout: &str) -> ProcessAttempt {
    ProcessAttempt::Completed {
        receipt: ProcessReceipt {
            provider: "model".into(),
            elapsed_mono_ns: 1,
            result: ProcResult {
                stdout: stdout.into(),
                stderr: "human-only".into(),
                exit_code: 0,
                truncated: false,
            },
        },
    }
}
fn sym(value: &str) -> Symbol {
    Symbol::new(value).unwrap()
}
fn bindings() -> AnsibleBindings {
    AnsibleBindings {
        root: ProjectRootRef::new("project/fixture").unwrap(),
        inventory_program: ProgramRef::new("program/ansible-inventory").unwrap(),
        config_program: ProgramRef::new("program/ansible-config").unwrap(),
        make_program: ProgramRef::new("program/make").unwrap(),
        operations: [(
            sym("deploy/perform"),
            OperationBinding {
                make_target: "perform".into(),
                target_assignment: None,
            },
        )]
        .into(),
        base_environment: BTreeMap::new(),
        callback_plugin: PrivateArtifactRef::new("artifact/callback").unwrap(),
        event_output: PrivateArtifactRef::new("artifact/events").unwrap(),
        human_output: PrivateArtifactRef::new("artifact/human").unwrap(),
        timeout_ms: 100,
    }
}
fn wire(
    run: &str,
    plan: &str,
    sequence: u32,
    prior: &str,
    status: &str,
    changed: bool,
    terminal: bool,
) -> (String, String) {
    let mut value = serde_json::json!({"schema":1,"run":run,"plan":plan,"sequence":sequence,"host":null,"task":null,"status":status,"changed":changed,"prior_hash":prior,"current_hash":"","terminal":terminal});
    let hash = hex(&Sha256::digest(serde_json::to_vec(&value).unwrap()));
    value["current_hash"] = serde_json::Value::String(hash.clone());
    (serde_json::to_string(&value).unwrap() + "\n", hash)
}

#[test]
fn discovery_is_one_fixed_request_and_discards_hostvars() {
    let raw = r#"{"_meta":{"hostvars":{"secret-host":{"password":"no"}}},"web":{"hosts":["a","a"],"children":[]}}"#;
    let port = Port {
        outcomes: Mutex::new(vec![completed(raw)]),
        requests: Mutex::default(),
    };
    let artifacts = Artifacts(Mutex::default());
    let mut site = AnsibleSite::new(&port, &artifacts, bindings());
    let (_, inventory) = site.discover().unwrap();
    assert_eq!(inventory.targets.len(), 1);
    let request = &port.requests.lock().unwrap()[0];
    assert_eq!(
        request.argv.iter().map(ArgAtom::as_str).collect::<Vec<_>>(),
        ["--list"]
    );
    let portable = serde_json::to_string(&inventory).unwrap();
    assert!(
        !portable.contains("hostvars")
            && !portable.contains("password")
            && !portable.contains("secret-host")
    );
}

#[test]
fn callback_decoder_fails_closed_for_chain_gaps_cross_run_and_trailing_data() {
    let (first, hash) = wire(
        "run/a",
        "plan/a",
        0,
        &"0".repeat(64),
        "accepted",
        false,
        false,
    );
    let (last, _) = wire("run/a", "plan/a", 1, &hash, "final", true, true);
    let valid = first.clone() + &last;
    assert_eq!(
        decode_events(valid.as_bytes(), &sym("run/a"), &sym("plan/a"), true)
            .unwrap()
            .state,
        RunState::Changed
    );
    assert!(decode_events(last.as_bytes(), &sym("run/a"), &sym("plan/a"), true).is_err());
    assert!(decode_events(valid.as_bytes(), &sym("run/b"), &sym("plan/a"), true).is_err());
    assert!(
        decode_events(
            (valid + &first).as_bytes(),
            &sym("run/a"),
            &sym("plan/a"),
            true
        )
        .is_err()
    );
}

#[test]
fn effective_config_accepts_current_ansible_env_rows() {
    let rows: Vec<ConfigRow> = serde_json::from_str(
        r#"[{"name":"CALLBACKS_ENABLED","origin":"env: ANSIBLE_CALLBACKS_ENABLED","value":["sim_estate_aggregate"]},{"name":"DEFAULT_LOAD_CALLBACK_PLUGINS","origin":"env: ANSIBLE_LOAD_CALLBACK_PLUGINS","value":true},{"GALAXY_SERVERS":{}}]"#,
    )
    .unwrap();
    assert!(config_list_contains(
        &rows,
        "CALLBACKS_ENABLED",
        "sim_estate_aggregate"
    ));
    assert!(config_bool(&rows, "DEFAULT_LOAD_CALLBACK_PLUGINS", true));
    assert!(!config_bool(&rows, "SHOW_PER_HOST_START", true));
}

#[test]
fn reconciliation_after_controller_loss_never_redispatches() {
    let inventory = r#"{"all":{"hosts":["node"],"children":[]}}"#;
    let config = r#"[{"name":"CALLBACKS_ENABLED","source":null,"value":["sim_estate_aggregate"]},{"name":"DEFAULT_LOAD_CALLBACK_PLUGINS","source":null,"value":true}]"#;
    let port = Port {
        outcomes: Mutex::new(vec![
            completed(inventory),
            completed(config),
            ProcessAttempt::UnknownAfterDispatch {
                evidence: sim_lib_exec::DispatchEvidence {
                    provider: "model".into(),
                    stage: "controller-death".into(),
                    detail: "lost".into(),
                },
            },
        ]),
        requests: Mutex::default(),
    };
    let artifacts = Artifacts(Mutex::default());
    let mut site = AnsibleSite::new(&port, &artifacts, bindings());
    let (_, inventory) = site.discover().unwrap();
    let operation = Operation {
        version: 1,
        exposure: sym("deploy/perform"),
        target: inventory.targets[0].id.clone(),
        parameters: BTreeMap::new(),
        mode: sim_estate_core::OperationMode::Change,
    };
    let project = ProjectFingerprint::of(b"fixture");
    let plan = site.plan(operation, project.clone()).unwrap();
    let run_id = format!("run/{}", &hex(&Sha256::digest(plan.id.as_str()))[..24]);
    let (accepted, hash) = wire(
        &run_id,
        plan.id.as_str(),
        0,
        &"0".repeat(64),
        "accepted",
        false,
        false,
    );
    let (final_line, _) = wire(&run_id, plan.id.as_str(), 1, &hash, "final", true, true);
    *artifacts.0.lock().unwrap() = (accepted + &final_line).into_bytes();
    let run = site
        .perform(
            &plan,
            &Approval {
                version: 1,
                plan: plan.id.clone(),
                project,
                granted: true,
            },
        )
        .unwrap();
    let before = site.dispatch_count();
    assert_eq!(site.reconcile(&run).unwrap().state, RunState::Changed);
    assert_eq!(site.dispatch_count(), before);
    let requests = port.requests.lock().unwrap();
    assert_eq!(
        requests[1]
            .argv
            .iter()
            .map(ArgAtom::as_str)
            .collect::<Vec<_>>(),
        ["dump", "--only-changed", "--format", "json"]
    );
    let request = &requests[2];
    assert_eq!(request.argv[0].as_str(), "perform");
    assert!(
        request
            .environment
            .iter()
            .all(|(name, _)| name.starts_with("ANSIBLE_") || name.starts_with("SIM_ESTATE_"))
    );
    assert!(
        !serde_json::to_string(&site.reconcile(&run).unwrap())
            .unwrap()
            .contains("human-only")
    );
}

#[test]
fn callback_asset_is_stdlib_and_never_serializes_result_data() {
    for required in [
        "hashlib",
        "json",
        "from ansible.plugins.callback import CallbackBase",
        "class CallbackModule(CallbackBase):",
        "SIM_ESTATE_EVENTS",
        "CALLBACK_TYPE = \"aggregate\"",
    ] {
        assert!(CALLBACK_PY.contains(required));
    }
    for forbidden in [
        "json.dumps(result._result",
        "exception",
        "diff",
        "facts",
        "task.args",
        "subprocess",
    ] {
        assert!(!CALLBACK_PY.contains(forbidden));
    }
}

#[test]
fn shared_provider_conformance_runs_against_sealed_fixture() {
    let inventory = r#"{"all":{"hosts":["node"],"children":[]},"_meta":{"hostvars":{"node":{"secret":"discard"}}}}"#;
    let config = r#"[{"name":"CALLBACKS_ENABLED","source":null,"value":["sim_estate_aggregate"]},{"name":"DEFAULT_LOAD_CALLBACK_PLUGINS","source":null,"value":true}]"#;
    let port = Port {
        outcomes: Mutex::new(vec![completed(inventory), completed(config), completed("")]),
        requests: Mutex::default(),
    };
    let artifacts = Artifacts(Mutex::default());
    let mut site = AnsibleSite::new(&port, &artifacts, bindings());
    let target = decode_inventory(inventory.as_bytes()).unwrap().targets[0]
        .id
        .clone();
    let operation = Operation {
        version: 1,
        exposure: sym("deploy/perform"),
        target,
        parameters: BTreeMap::new(),
        mode: sim_estate_core::OperationMode::Change,
    };
    let operation_hash = hex(&Sha256::digest(serde_json::to_vec(&operation).unwrap()));
    let plan = format!("plan/{}", &operation_hash[..24]);
    let run = format!("run/{}", &hex(&Sha256::digest(&plan))[..24]);
    let (accepted, hash) = wire(&run, &plan, 0, &"0".repeat(64), "accepted", false, false);
    let (final_line, _) = wire(&run, &plan, 1, &hash, "final", false, true);
    *artifacts.0.lock().unwrap() = (accepted + &final_line).into_bytes();
    let outcome = sim_estate_core::conformance_smoke(
        &mut site,
        operation,
        ProjectFingerprint::of(b"fixture"),
    )
    .unwrap();
    assert_eq!(outcome.state, RunState::Unchanged);
    assert_eq!(site.dispatch_count(), 3);
}
