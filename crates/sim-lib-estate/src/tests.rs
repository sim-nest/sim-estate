use super::*;
use sim_lib_estate_book::MemoryTable;
use sim_site_estate_model::{Fault, ModelEstate};
struct Time(u64);
impl Clock for Time {
    fn now(&self) -> u64 {
        self.0
    }
}
#[derive(Default)]
struct Gate {
    calls: usize,
}
impl EffectGate for Gate {
    fn perform<R>(
        &mut self,
        _: &EffectRequest,
        f: impl FnOnce() -> Result<R, EstateError>,
    ) -> Result<R, Error> {
        self.calls += 1;
        f().map_err(Into::into)
    }
}
fn sym(s: &str) -> Symbol {
    Symbol::new(s).unwrap()
}
fn op() -> Operation {
    Operation {
        version: 1,
        exposure: sym("service/restart"),
        target: sym("fleet/web"),
        parameters: BTreeMap::new(),
        mode: sim_estate_core::OperationMode::Change,
    }
}
fn planned(organ: &Organ<MemoryTable>, provider: &mut ModelEstate) -> Planned {
    organ
        .plan(
            provider,
            op(),
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
        .unwrap()
}

#[test]
fn immutable_plan_is_canonical_and_single_discovery() {
    let organ = Organ::new(MemoryTable::default());
    let mut provider = ModelEstate::fixture();
    let first = planned(&organ, &mut provider);
    let second = planned(&organ, &mut provider);
    assert_eq!(first.key, second.key);
    assert_eq!(first.plan.targets, vec![sym("fleet/web")]);
}
#[test]
fn one_approval_and_target_have_one_winner() {
    let organ = Organ::new(MemoryTable::default());
    let mut provider = ModelEstate::fixture();
    let plan = planned(&organ, &mut provider);
    let approval = organ.review("a", &plan, "reviewer", "r1").unwrap();
    let mut gate = Gate::default();
    assert!(
        organ
            .apply(
                &mut provider,
                &mut gate,
                &Time(0),
                &plan,
                "a",
                approval,
                "r1"
            )
            .is_ok()
    );
    assert!(organ.book.consume_approval("a", "r1").is_err());
}
#[test]
fn unknown_dispatch_quarantines_and_expiry_does_not_release() {
    let organ = Organ::new(MemoryTable::default());
    let mut provider = ModelEstate::fixture();
    let plan = planned(&organ, &mut provider);
    let approval = organ.review("a", &plan, "reviewer", "r").unwrap();
    provider.inject(Fault::UnknownAfterDispatch);
    let out = organ
        .apply(
            &mut provider,
            &mut Gate::default(),
            &Time(0),
            &plan,
            "a",
            approval,
            "r",
        )
        .unwrap();
    assert_eq!(out.state, ApplyState::Quarantined);
    assert!(
        organ
            .book
            .lease_state("fleet/web")
            .unwrap()
            .unwrap()
            .quarantined
    );
    assert!(organ.renew("r", &plan.plan.targets, &Time(99), 10).is_err());
}
#[test]
fn stale_project_is_a_typed_gated_refusal() {
    let organ = Organ::new(MemoryTable::default());
    let mut provider = ModelEstate::fixture();
    let plan = planned(&organ, &mut provider);
    let approval = organ.review("a", &plan, "reviewer", "r").unwrap();
    provider.revise_project(b"changed");
    let mut gate = Gate::default();
    assert!(matches!(
        organ.apply(
            &mut provider,
            &mut gate,
            &Time(0),
            &plan,
            "a",
            approval,
            "r"
        ),
        Err(Error::Estate(EstateError::StalePlan))
    ));
    assert_eq!(gate.calls, 1);
}
#[test]
fn cassette_reconcile_never_calls_provider() {
    struct Art;
    impl ArtifactReader for Art {
        fn reconcile(&self, _: &str) -> Result<Option<RunState>, Error> {
            Ok(Some(RunState::Changed))
        }
    }
    let organ = Organ::new(MemoryTable::default());
    let plan = organ.book.intern(&"p").unwrap();
    organ.book.append("r", EventKind::Opened { plan }).unwrap();
    organ.book.append("r", EventKind::DispatchIntent).unwrap();
    assert_eq!(organ.reconcile("r", &Art).unwrap(), RunState::Changed);
}
