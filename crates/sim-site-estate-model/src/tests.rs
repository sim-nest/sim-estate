use super::*;
use sim_estate_core::{OperationMode, conformance_smoke};
fn operation() -> Operation {
    Operation {
        version: 1,
        exposure: sym("service/restart"),
        target: sym("fleet/web"),
        parameters: BTreeMap::new(),
        mode: OperationMode::Change,
    }
}
fn project() -> ProjectFingerprint {
    ProjectFingerprint::of(b"fixture-project-v1")
}
#[test]
fn shared_conformance_passes_changed_and_unchanged() {
    for outcome in [DispatchOutcome::Changed, DispatchOutcome::Unchanged] {
        let mut model = ModelEstate::fixture();
        model.set_outcome(outcome);
        assert!(conformance_smoke(&mut model, operation(), project()).is_ok());
    }
}
#[test]
fn preview_unsupported_is_conformant() {
    let mut model = ModelEstate::fixture();
    model.inject(Fault::PreviewUnsupported);
    assert!(conformance_smoke(&mut model, operation(), project()).is_ok());
}
#[test]
fn every_injected_fault_is_typed() {
    let expected = [
        (Fault::TargetDisappears, EstateError::TargetDisappeared),
        (Fault::StalePlan, EstateError::StalePlan),
        (Fault::NotDispatched, EstateError::NotDispatched),
        (
            Fault::UnknownAfterDispatch,
            EstateError::UnknownAfterDispatch,
        ),
        (Fault::MalformedEvent, EstateError::MalformedEvent),
        (Fault::VerificationFails, EstateError::VerificationFailed),
    ];
    for (fault, wanted) in expected {
        let mut model = ModelEstate::fixture();
        model.inject(fault);
        assert_eq!(
            conformance_smoke(&mut model, operation(), project()).unwrap_err(),
            wanted
        );
    }
}
#[test]
fn target_disappearance_after_plan_and_stale_revision_fail() {
    let mut model = ModelEstate::fixture();
    let plan = model.plan(operation(), project()).unwrap();
    model.replace_inventory(vec![]);
    let approval = Approval {
        version: 1,
        plan: plan.id.clone(),
        project: project(),
        granted: true,
    };
    assert_eq!(model.perform(&plan, &approval), Err(EstateError::StalePlan));
}
#[test]
fn cancellation_and_reconciliation_are_deterministic() {
    let mut model = ModelEstate::fixture();
    let plan = model.plan(operation(), project()).unwrap();
    let approval = Approval {
        version: 1,
        plan: plan.id.clone(),
        project: project(),
        granted: true,
    };
    let run = model.perform(&plan, &approval).unwrap();
    model.advance(10);
    let cancelled = model.cancel(&run).unwrap();
    assert_eq!(cancelled.state, RunState::Cancelled);
    assert_eq!(model.reconcile(&run).unwrap().observed_at, 10);
}
#[test]
fn scripted_revision_and_events_are_repeatable() {
    let mut model = ModelEstate::fixture();
    model.revise_project(b"revision-2");
    model.script_events(vec![EventKind::Accepted, EventKind::Unchanged]);
    assert_ne!(model.project, project());
    assert_eq!(model.events.len(), 2);
}
