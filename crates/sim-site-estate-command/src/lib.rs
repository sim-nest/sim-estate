#![forbid(unsafe_code)]
//! Pure compilation of closed estate operations into sealed process requests.
// conformance: command requests contain only reviewed programs, roots, atoms, and budgets.

use sha2::{Digest, Sha256};
use sim_estate_core::{Operation, ProjectFingerprint, Symbol};
use sim_estate_project::{Catalog, CompileError};
use sim_lib_exec::{
    ArgAtom, ProcessBudget, ProcessRequest, ProgramRef, ProjectRootRef, SealedBindings,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandBindings {
    pub root: ProjectRootRef,
    pub programs: std::collections::BTreeMap<Symbol, ProgramRef>,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CommandError {
    #[error("operation was not compiled by this catalog")]
    InvalidOperation,
    #[error("manifest did not bind program: {0:?}")]
    UnboundProgram(Symbol),
    #[error("invalid sealed process atom")]
    InvalidAtom,
}

pub fn compile_request(
    catalog: &Catalog,
    operation: &Operation,
    bindings: &CommandBindings,
) -> Result<ProcessRequest, CommandError> {
    let exposure = catalog
        .exposure(&operation.exposure)
        .ok_or(CommandError::InvalidOperation)?;
    let argv = catalog
        .render_arguments(operation)
        .map_err(|_: CompileError| CommandError::InvalidOperation)?
        .into_iter()
        .map(|value| ArgAtom::new(value).map_err(|_| CommandError::InvalidAtom))
        .collect::<Result<Vec<_>, _>>()?;
    let program = bindings
        .programs
        .get(&exposure.program_ref)
        .cloned()
        .ok_or_else(|| CommandError::UnboundProgram(exposure.program_ref.clone()))?;
    Ok(ProcessRequest {
        program,
        argv,
        root: bindings.root.clone(),
        environment: SealedBindings::empty(),
        private_artifacts: vec![],
        budget: ProcessBudget {
            timeout_ms: bindings.timeout_ms,
            max_output_bytes: bindings.max_output_bytes,
            stdin: None,
        },
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectInput<'a> {
    pub name: &'a str,
    pub digest: [u8; 32],
}

/// Binds repository and policy state without retaining project file content.
#[must_use]
pub fn fingerprint_project(
    head: &str,
    tracked_changes: &[ProjectInput<'_>],
    untracked_inputs: &[ProjectInput<'_>],
    effective_config_digest: [u8; 32],
    exposure_digest: [u8; 32],
) -> ProjectFingerprint {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, b"head", head.as_bytes());
    for (kind, values) in [
        (b"tracked".as_slice(), tracked_changes),
        (b"untracked", untracked_inputs),
    ] {
        for value in values {
            hash_field(&mut hasher, kind, value.name.as_bytes());
            hash_field(&mut hasher, b"digest", &value.digest);
        }
    }
    hash_field(&mut hasher, b"config", &effective_config_digest);
    hash_field(&mut hasher, b"exposure", &exposure_digest);
    ProjectFingerprint {
        version: 1,
        digest: hasher.finalize().into(),
    }
}

fn hash_field(hasher: &mut Sha256, tag: &[u8], value: &[u8]) {
    hasher.update((tag.len() as u64).to_be_bytes());
    hasher.update(tag);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_estate_core::{OperationMode, Value};
    use std::collections::BTreeMap;
    fn sym(v: &str) -> Symbol {
        Symbol::new(v).unwrap()
    }
    fn catalog() -> Catalog {
        Catalog::compile_json(br#"{"version":1,"exposures":[{"id":"op/run","provider":"provider/command","program_ref":"program/make","arguments":[{"literal":"deploy","parameter":"value/name"}],"parameters":[{"id":"value/name","shape":{"symbol":{"namespace":"value/"}},"encoder":"symbol-value"}],"mode":"Change","capability":"estate/change","risk_floor":"risk/medium","preview":true,"verification":"verify/deploy","callback":"progress-events","bounds":{"max_targets":1,"max_events":8,"max_duration_ticks":10,"max_value_bytes":64}}]}"#).unwrap()
    }
    #[test]
    fn emits_only_bound_program_and_whole_atoms() {
        let catalog = catalog();
        let mut parameters = BTreeMap::new();
        parameters.insert(sym("value/name"), Value::Symbol(sym("value/web")));
        let operation = Operation {
            version: 1,
            exposure: sym("op/run"),
            target: sym("host/a"),
            parameters,
            mode: OperationMode::Change,
        };
        let bindings = CommandBindings {
            root: ProjectRootRef::new("project/fixture").unwrap(),
            programs: [(
                sym("program/make"),
                ProgramRef::new("program/make").unwrap(),
            )]
            .into(),
            timeout_ms: 10,
            max_output_bytes: 64,
        };
        let request = compile_request(&catalog, &operation, &bindings).unwrap();
        assert_eq!(
            request.argv.iter().map(ArgAtom::as_str).collect::<Vec<_>>(),
            ["deploy", "value/web"]
        );
        assert!(request.environment.iter().next().is_none());
    }
    #[test]
    fn fingerprint_changes_for_every_execution_input_class() {
        let d = |v: &[u8]| Sha256::digest(v).into();
        let base = fingerprint_project(
            "a",
            &[ProjectInput {
                name: "Makefile",
                digest: d(b"a"),
            }],
            &[],
            d(b"config"),
            d(b"exposure"),
        );
        assert_ne!(
            base,
            fingerprint_project(
                "b",
                &[ProjectInput {
                    name: "Makefile",
                    digest: d(b"a")
                }],
                &[],
                d(b"config"),
                d(b"exposure")
            )
        );
        assert_ne!(
            base,
            fingerprint_project(
                "a",
                &[ProjectInput {
                    name: "Makefile",
                    digest: d(b"b")
                }],
                &[],
                d(b"config"),
                d(b"exposure")
            )
        );
        assert_ne!(
            base,
            fingerprint_project(
                "a",
                &[ProjectInput {
                    name: "Makefile",
                    digest: d(b"a")
                }],
                &[ProjectInput {
                    name: "vars/new.yml",
                    digest: d(b"x")
                }],
                d(b"config"),
                d(b"exposure")
            )
        );
    }
}
