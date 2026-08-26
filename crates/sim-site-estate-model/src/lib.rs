#![forbid(unsafe_code)]
//! Deterministic estate provider site.
// conformance: the model provider exercises every dispatch and verification boundary.

use sim_estate_core::{
    Approval, EstateError, EstateProvider, Event, EventKind, Operation, Outcome, Plan, Preview,
    ProjectFingerprint, ProviderCard, Reconciliation, Run, RunState, SCHEMA_VERSION,
    SanitizedInventory, Symbol, Target, Verification,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Fault {
    #[default]
    None,
    StalePlan,
    TargetDisappears,
    PreviewUnsupported,
    VerificationFails,
    NotDispatched,
    UnknownAfterDispatch,
    MalformedEvent,
    Cancelled,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DispatchOutcome {
    #[default]
    Changed,
    Unchanged,
}

pub struct ModelEstate {
    tick: u64,
    revision: u64,
    project: ProjectFingerprint,
    targets: Vec<Target>,
    events: Vec<EventKind>,
    fault: Fault,
    outcome: DispatchOutcome,
    dispatched: bool,
}
impl Default for ModelEstate {
    fn default() -> Self {
        Self::fixture()
    }
}
impl ModelEstate {
    #[must_use]
    pub fn fixture() -> Self {
        Self {
            tick: 0,
            revision: 1,
            project: ProjectFingerprint::of(b"fixture-project-v1"),
            targets: vec![Target {
                version: 1,
                id: sym("fleet/web"),
                labels: BTreeMap::new(),
            }],
            events: vec![EventKind::Accepted, EventKind::Progress, EventKind::Changed],
            fault: Fault::None,
            outcome: DispatchOutcome::Changed,
            dispatched: false,
        }
    }
    pub fn inject(&mut self, fault: Fault) {
        self.fault = fault;
    }
    pub fn set_outcome(&mut self, outcome: DispatchOutcome) {
        self.outcome = outcome;
    }
    pub fn script_events(&mut self, events: Vec<EventKind>) {
        self.events = events;
    }
    pub fn advance(&mut self, ticks: u64) {
        self.tick = self.tick.saturating_add(ticks);
    }
    pub fn revise_project(&mut self, content: &[u8]) {
        self.project = ProjectFingerprint::of(content);
    }
    pub fn replace_inventory(&mut self, targets: Vec<Target>) {
        self.targets = targets;
        self.revision += 1;
    }
}
fn sym(value: &str) -> Symbol {
    Symbol::new(value).expect("model symbols are static and valid")
}

impl EstateProvider for ModelEstate {
    fn discover(&mut self) -> Result<(ProviderCard, SanitizedInventory), EstateError> {
        Ok((
            ProviderCard {
                version: 1,
                provider: sym("provider/model"),
                capabilities: vec![sym("estate/inspect"), sym("estate/change")],
                max_targets: 1024,
            },
            SanitizedInventory {
                version: 1,
                revision: self.revision,
                targets: self.targets.clone(),
            },
        ))
    }
    fn plan(
        &mut self,
        operation: Operation,
        project: ProjectFingerprint,
    ) -> Result<Plan, EstateError> {
        if self.fault == Fault::TargetDisappears
            || !self.targets.iter().any(|t| t.id == operation.target)
        {
            return Err(EstateError::TargetDisappeared);
        }
        Ok(Plan {
            version: 1,
            id: sym("plan/model-1"),
            operation,
            inventory_revision: self.revision,
            project,
            risk: sym("risk/medium"),
            preview: None,
        })
    }
    fn preview(&mut self, _plan: &Plan) -> Result<Preview, EstateError> {
        if self.fault == Fault::PreviewUnsupported {
            return Err(EstateError::Unsupported(sym("preview")));
        }
        Ok(Preview {
            changed_targets: u32::from(self.outcome == DispatchOutcome::Changed),
            notices: vec![],
        })
    }
    fn perform(&mut self, plan: &Plan, approval: &Approval) -> Result<Run, EstateError> {
        if self.fault == Fault::StalePlan
            || plan.inventory_revision != self.revision
            || plan.project != self.project
            || approval.project != plan.project
        {
            return Err(EstateError::StalePlan);
        }
        if self.fault == Fault::TargetDisappears
            || !self.targets.iter().any(|t| t.id == plan.operation.target)
        {
            return Err(EstateError::TargetDisappeared);
        }
        if self.fault == Fault::NotDispatched {
            return Err(EstateError::NotDispatched);
        }
        self.dispatched = true;
        if self.fault == Fault::UnknownAfterDispatch {
            return Err(EstateError::UnknownAfterDispatch);
        }
        Ok(Run {
            version: 1,
            id: sym("run/model-1"),
            plan: plan.id.clone(),
            dispatched_at: self.tick,
        })
    }
    fn events(&mut self, _run: &Run, limit: u32) -> Result<Vec<Event>, EstateError> {
        if self.fault == Fault::MalformedEvent {
            return Ok(vec![Event {
                version: 1,
                sequence: 0,
                at: self.tick,
                kind: EventKind::Malformed,
            }]);
        }
        Ok(self
            .events
            .iter()
            .take(limit as usize)
            .enumerate()
            .map(|(i, kind)| Event {
                version: 1,
                sequence: u32::try_from(i).expect("event count was bounded by the u32 limit"),
                at: self.tick + i as u64,
                kind: kind.clone(),
            })
            .collect())
    }
    fn verify(&mut self, run: &Run) -> Result<Verification, EstateError> {
        if self.fault == Fault::VerificationFails {
            return Err(EstateError::VerificationFailed);
        }
        Ok(Verification {
            version: 1,
            run: run.id.clone(),
            passed: true,
            checks: vec![sym("check/model-state")],
        })
    }
    fn reconcile(&mut self, run: &Run) -> Result<Reconciliation, EstateError> {
        let state = if !self.dispatched {
            RunState::NotDispatched
        } else if self.fault == Fault::UnknownAfterDispatch {
            RunState::UnknownAfterDispatch
        } else if self.fault == Fault::Cancelled {
            RunState::Cancelled
        } else {
            match self.outcome {
                DispatchOutcome::Changed => RunState::Changed,
                DispatchOutcome::Unchanged => RunState::Unchanged,
            }
        };
        Ok(Reconciliation {
            version: 1,
            run: run.id.clone(),
            state,
            observed_at: self.tick,
        })
    }
    fn cancel(&mut self, run: &Run) -> Result<Outcome, EstateError> {
        self.fault = Fault::Cancelled;
        Ok(Outcome {
            version: SCHEMA_VERSION,
            run: Some(run.id.clone()),
            state: RunState::Cancelled,
            verification: None,
            reconciliation: Some(self.reconcile(run)?),
        })
    }
}

#[cfg(test)]
mod tests {
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
}
