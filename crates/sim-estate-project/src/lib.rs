#![forbid(unsafe_code)]
//! Strict, pure exposure compilation.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sim_estate_core::{
    Operation, OperationMode, ProjectFingerprint, SCHEMA_VERSION, Symbol, Value,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParameterShape {
    Bool,
    Integer {
        min: i64,
        max: i64,
    },
    Text {
        max_bytes: u32,
    },
    Symbol {
        namespace: Symbol,
    },
    List {
        max_items: u32,
        item: Box<ParameterShape>,
    },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TypedEncoder {
    BoolFlag,
    Decimal,
    Utf8Value,
    SymbolValue,
    Repeated,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub id: Symbol,
    pub shape: ParameterShape,
    pub encoder: TypedEncoder,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArgumentTemplate {
    pub literal: String,
    pub parameter: Option<Symbol>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CallbackPolicy {
    None,
    ProgressEvents,
    CompletionOnly,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub max_targets: u32,
    pub max_events: u32,
    pub max_duration_ticks: u64,
    pub max_value_bytes: u32,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exposure {
    pub id: Symbol,
    pub provider: Symbol,
    pub program_ref: Symbol,
    pub arguments: Vec<ArgumentTemplate>,
    pub parameters: Vec<Parameter>,
    pub mode: OperationMode,
    pub capability: Symbol,
    pub risk_floor: Symbol,
    pub preview: bool,
    pub verification: Symbol,
    pub callback: CallbackPolicy,
    pub bounds: Bounds,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub version: u16,
    pub exposures: Vec<Exposure>,
}
#[derive(Clone, Debug)]
pub struct Catalog {
    fingerprint: ProjectFingerprint,
    exposures: BTreeMap<Symbol, Exposure>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CompileError {
    #[error("invalid project syntax: {0}")]
    Syntax(String),
    #[error("duplicate exposure or parameter id: {0:?}")]
    Duplicate(Symbol),
    #[error("template references undeclared parameter: {0:?}")]
    UnknownParameter(Symbol),
    #[error("literal contains forbidden delimiter or control data")]
    UnsafeLiteral,
    #[error("value does not satisfy shape: {0:?}")]
    Shape(Symbol),
    #[error("unknown exposure: {0:?}")]
    UnknownExposure(Symbol),
    #[error("bound is zero or exceeds the portable maximum")]
    InvalidBounds,
}

impl Catalog {
    pub fn compile_json(input: &[u8]) -> Result<Self, CompileError> {
        let project: Project =
            serde_json::from_slice(input).map_err(|e| CompileError::Syntax(e.to_string()))?;
        if project.version != SCHEMA_VERSION {
            return Err(CompileError::Syntax("unsupported version".into()));
        }
        let mut exposures = BTreeMap::new();
        for exposure in project.exposures {
            validate_exposure(&exposure)?;
            let id = exposure.id.clone();
            if exposures.insert(id.clone(), exposure).is_some() {
                return Err(CompileError::Duplicate(id));
            }
        }
        let canonical = serde_json::to_vec(&Project {
            version: project.version,
            exposures: exposures.values().cloned().collect(),
        })
        .map_err(|e| CompileError::Syntax(e.to_string()))?;
        Ok(Self {
            fingerprint: ProjectFingerprint {
                version: SCHEMA_VERSION,
                digest: Sha256::digest(canonical).into(),
            },
            exposures,
        })
    }
    #[must_use]
    pub fn fingerprint(&self) -> ProjectFingerprint {
        self.fingerprint.clone()
    }
    pub fn operation(
        &self,
        exposure: &Symbol,
        target: Symbol,
        parameters: BTreeMap<Symbol, Value>,
    ) -> Result<Operation, CompileError> {
        let definition = self
            .exposures
            .get(exposure)
            .ok_or_else(|| CompileError::UnknownExposure(exposure.clone()))?;
        if parameters.len() != definition.parameters.len() {
            return Err(CompileError::Shape(exposure.clone()));
        }
        for parameter in &definition.parameters {
            let value = parameters
                .get(&parameter.id)
                .ok_or_else(|| CompileError::Shape(parameter.id.clone()))?;
            if !matches_shape(value, &parameter.shape) {
                return Err(CompileError::Shape(parameter.id.clone()));
            }
        }
        Ok(Operation {
            version: SCHEMA_VERSION,
            exposure: exposure.clone(),
            target,
            parameters,
            mode: definition.mode.clone(),
        })
    }
    #[must_use]
    pub fn exposure(&self, id: &Symbol) -> Option<&Exposure> {
        self.exposures.get(id)
    }

    pub fn render_arguments(&self, operation: &Operation) -> Result<Vec<String>, CompileError> {
        let exposure = self
            .exposures
            .get(&operation.exposure)
            .ok_or_else(|| CompileError::UnknownExposure(operation.exposure.clone()))?;
        let mut rendered = Vec::new();
        for template in &exposure.arguments {
            rendered.push(template.literal.clone());
            if let Some(id) = &template.parameter {
                let value = operation
                    .parameters
                    .get(id)
                    .ok_or_else(|| CompileError::Shape(id.clone()))?;
                render_value(value, &mut rendered);
            }
        }
        Ok(rendered)
    }
}

fn render_value(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::Bool(value) => output.push(value.to_string()),
        Value::Integer(value) => output.push(value.to_string()),
        Value::Text(value) => output.push(value.clone()),
        Value::Symbol(value) => output.push(value.as_str().to_owned()),
        Value::List(values) => values.iter().for_each(|value| render_value(value, output)),
    }
}

fn validate_exposure(value: &Exposure) -> Result<(), CompileError> {
    let b = &value.bounds;
    if b.max_targets == 0
        || b.max_targets > 1024
        || b.max_events == 0
        || b.max_events > 4096
        || b.max_duration_ticks == 0
        || b.max_value_bytes == 0
        || b.max_value_bytes > 65_536
    {
        return Err(CompileError::InvalidBounds);
    }
    let mut ids = BTreeSet::new();
    for parameter in &value.parameters {
        if !ids.insert(parameter.id.clone()) {
            return Err(CompileError::Duplicate(parameter.id.clone()));
        }
    }
    for argument in &value.arguments {
        if argument.literal.len() > 256
            || argument.literal.starts_with(['-', '.'])
            || argument.literal.chars().any(|c| {
                !c.is_ascii()
                    || c.is_whitespace()
                    || c.is_control()
                    || matches!(
                        c,
                        ';' | '|'
                            | '&'
                            | '`'
                            | '$'
                            | '='
                            | '*'
                            | '?'
                            | '\\'
                            | '/'
                            | ':'
                            | '.'
                            | '\''
                            | '"'
                    )
            })
        {
            return Err(CompileError::UnsafeLiteral);
        }
        if let Some(id) = &argument.parameter
            && !ids.contains(id)
        {
            return Err(CompileError::UnknownParameter(id.clone()));
        }
    }
    Ok(())
}
fn matches_shape(value: &Value, shape: &ParameterShape) -> bool {
    match (value, shape) {
        (Value::Bool(_), ParameterShape::Bool) => true,
        (Value::Integer(v), ParameterShape::Integer { min, max }) => v >= min && v <= max,
        (Value::Text(v), ParameterShape::Text { max_bytes }) => {
            v.len() <= *max_bytes as usize && !v.contains('\0')
        }
        (Value::Symbol(v), ParameterShape::Symbol { namespace }) => {
            v.as_str().starts_with(namespace.as_str())
        }
        (Value::List(values), ParameterShape::List { max_items, item }) => {
            values.len() <= *max_items as usize && values.iter().all(|v| matches_shape(v, item))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
