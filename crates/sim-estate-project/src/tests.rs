use super::*;
use sim_estate_core::{OperationMode, Symbol, Value};
use std::collections::BTreeMap;
fn sym(v: &str) -> Symbol {
    Symbol::new(v).unwrap()
}
fn valid() -> Exposure {
    Exposure {
        id: sym("service/restart"),
        provider: sym("provider/model"),
        program_ref: sym("program/service-control"),
        arguments: vec![ArgumentTemplate {
            literal: "restart".into(),
            parameter: Some(sym("service/name")),
        }],
        parameters: vec![Parameter {
            id: sym("service/name"),
            shape: ParameterShape::Symbol {
                namespace: sym("service/"),
            },
            encoder: TypedEncoder::SymbolValue,
        }],
        mode: OperationMode::Change,
        capability: sym("estate/change"),
        risk_floor: sym("risk/medium"),
        preview: true,
        verification: sym("verify/service"),
        callback: CallbackPolicy::ProgressEvents,
        bounds: Bounds {
            max_targets: 10,
            max_events: 20,
            max_duration_ticks: 100,
            max_value_bytes: 64,
        },
    }
}
fn json(exposures: Vec<Exposure>) -> Vec<u8> {
    serde_json::to_vec(&Project {
        version: 1,
        exposures,
    })
    .unwrap()
}
#[test]
fn compiles_closed_operation() {
    let catalog = Catalog::compile_json(&json(vec![valid()])).unwrap();
    let mut values = BTreeMap::new();
    values.insert(sym("service/name"), Value::Symbol(sym("service/web")));
    assert!(
        catalog
            .operation(&sym("service/restart"), sym("fleet/web"), values)
            .is_ok()
    );
}
#[test]
fn duplicate_ids_fail() {
    assert!(matches!(
        Catalog::compile_json(&json(vec![valid(), valid()])),
        Err(CompileError::Duplicate(_))
    ));
    let mut e = valid();
    e.parameters.push(e.parameters[0].clone());
    assert!(matches!(
        Catalog::compile_json(&json(vec![e])),
        Err(CompileError::Duplicate(_))
    ));
}
#[test]
fn unknown_fields_fail() {
    let mut text = String::from_utf8(json(vec![valid()])).unwrap();
    text = text.replacen("{\"version\":1", "{\"version\":1,\"playbook\":\"x\"", 1);
    assert!(Catalog::compile_json(text.as_bytes()).is_err());
}
#[test]
fn command_and_target_injection_are_unrepresentable() {
    for payload in [
        "sh -c",
        "make target",
        "x=y",
        "--extra-vars",
        "site.yml",
        "module/name",
        "../path",
        "fleet/*",
        "x;y",
        "x|y",
        "x\0y",
        "påth",
        "$(id)",
        "`id`",
    ] {
        let mut e = valid();
        e.arguments[0].literal = payload.into();
        assert!(
            Catalog::compile_json(&json(vec![e])).is_err(),
            "accepted {payload:?}"
        );
    }
}
#[test]
fn shaped_values_reject_nul_and_wrong_namespace() {
    let catalog = Catalog::compile_json(&json(vec![valid()])).unwrap();
    for value in [Value::Text("x\0y".into()), Value::Symbol(sym("program/sh"))] {
        let mut values = BTreeMap::new();
        values.insert(sym("service/name"), value);
        assert!(
            catalog
                .operation(&sym("service/restart"), sym("fleet/web"), values)
                .is_err()
        );
    }
}
