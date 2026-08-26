// conformance: controller acceptance distinguishes safe refusal from post-dispatch quarantine.

use std::collections::BTreeMap;

use sim_estate_core::{EstateError, Operation, OperationMode, ProjectFingerprint, Symbol};
use sim_lib_estate_book::{Key, MemoryTable};
use sim_site_estate_model::{Fault, ModelEstate};

use crate::{ApplyState, Clock, EffectGate, EffectRequest, Error, Organ, Policy};

struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> u64 {
        0
    }
}

struct Gate;

impl EffectGate for Gate {
    fn perform<R>(
        &mut self,
        _: &EffectRequest,
        action: impl FnOnce() -> Result<R, EstateError>,
    ) -> Result<R, Error> {
        action().map_err(Into::into)
    }
}

fn apply_with_fault(fault: Fault) -> Result<ApplyState, Error> {
    let organ = Organ::new(MemoryTable::default());
    let mut provider = ModelEstate::fixture();
    let planned = organ.plan(
        &mut provider,
        Operation {
            version: 1,
            exposure: Symbol::new("service/restart").unwrap(),
            target: Symbol::new("fleet/web").unwrap(),
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
        "acceptance".into(),
        0,
    )?;
    let approval = organ.review("approval", &planned, "reviewer", "run")?;
    provider.inject(fault);
    organ
        .apply(
            &mut provider,
            &mut Gate,
            &FixedClock,
            &planned,
            "approval",
            approval,
            "run",
        )
        .map(|outcome| outcome.state)
}

#[test]
fn refusal_is_retryable_only_before_dispatch() {
    assert!(matches!(
        apply_with_fault(Fault::NotDispatched),
        Err(Error::Estate(EstateError::NotDispatched))
    ));
    assert_eq!(
        apply_with_fault(Fault::UnknownAfterDispatch).unwrap(),
        ApplyState::Quarantined
    );
}
