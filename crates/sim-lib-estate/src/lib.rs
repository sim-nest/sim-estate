#![forbid(unsafe_code)]
//! Guarded, durable estate plan/apply/reconcile composition.

use serde::{Deserialize, Serialize};
use sim_estate_core::{
    Approval, EstateError, EstateProvider, Operation, Plan, Preview, ProjectFingerprint,
    ProviderCard, RunState, SanitizedInventory, Symbol, Verification,
};
use sim_lib_estate_book::{ApprovalUse, Book, BookError, EventKind, Key, LeaseRecord, Table};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub preview: bool,
    pub verify: bool,
    pub ttl: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ImmutablePlan {
    pub provider: ProviderCard,
    pub inventory: SanitizedInventory,
    pub project: ProjectFingerprint,
    pub operation: Operation,
    pub targets: Vec<Symbol>,
    pub values: BTreeMap<Symbol, sim_estate_core::Value>,
    pub program_card: Key,
    pub provider_card: Key,
    pub policy: Policy,
    pub nonce: String,
    pub risk: Symbol,
    pub expires_at: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Planned {
    pub key: Key,
    pub plan: ImmutablePlan,
    pub preview: Option<Preview>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EffectRequest {
    pub plan: Key,
    pub provider: Symbol,
    pub operation: Symbol,
    pub project: ProjectFingerprint,
    pub inventory: Key,
    pub targets: Vec<Symbol>,
    pub approval: Key,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplyState {
    Verified,
    ProcessCompleteUnverified,
    Quarantined,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplyOutcome {
    pub run: String,
    pub state: ApplyState,
    pub verification: Option<Verification>,
}
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Book(#[from] BookError),
    #[error(transparent)]
    Estate(#[from] EstateError),
    #[error("refused: {reason}; observed={observed}")]
    Refused { reason: String, observed: String },
    #[error("effect gate: {0}")]
    Gate(String),
}
pub trait Clock {
    fn now(&self) -> u64;
}
pub trait EffectGate {
    fn perform<R>(
        &mut self,
        request: &EffectRequest,
        action: impl FnOnce() -> Result<R, EstateError>,
    ) -> Result<R, Error>;
}
pub trait ArtifactReader {
    fn reconcile(&self, run: &str) -> Result<Option<RunState>, Error>;
}

pub struct Organ<T> {
    pub book: Book<T>,
}
impl<T: Table> Organ<T> {
    pub fn new(table: T) -> Self {
        Self {
            book: Book::new(table),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn plan<P: EstateProvider>(
        &self,
        provider: &mut P,
        operation: Operation,
        project: ProjectFingerprint,
        program_card: Key,
        policy: Policy,
        nonce: String,
        now: u64,
    ) -> Result<Planned, Error> {
        let (card, mut inventory) = provider.discover()?;
        inventory.targets.sort_by(|a, b| a.id.cmp(&b.id));
        let targets = inventory
            .targets
            .iter()
            .filter(|t| t.id == operation.target)
            .map(|t| t.id.clone())
            .collect::<Vec<_>>();
        if targets.is_empty() {
            return Err(EstateError::TargetDisappeared.into());
        }
        let provider_card = self.book.intern(&card)?;
        let risk = Symbol::new(
            if matches!(operation.mode, sim_estate_core::OperationMode::Change) {
                "risk/high"
            } else {
                "risk/read"
            },
        )?;
        let plan = ImmutablePlan {
            provider: card,
            inventory,
            project,
            values: operation.parameters.clone(),
            operation,
            targets,
            program_card,
            provider_card,
            policy,
            nonce,
            risk,
            expires_at: now.saturating_add(policy.ttl),
        };
        let key = self.book.intern(&plan)?;
        let provider_plan = to_provider_plan(&key, &plan)?;
        let preview = if policy.preview {
            match provider.preview(&provider_plan) {
                Ok(p) => Some(p),
                Err(EstateError::Unsupported(_)) => None,
                Err(e) => return Err(e.into()),
            }
        } else {
            None
        };
        Ok(Planned { key, plan, preview })
    }
    pub fn review(
        &self,
        id: &str,
        planned: &Planned,
        reviewer: &str,
        run: &str,
    ) -> Result<Key, Error> {
        let key = self.book.intern(&(id, &planned.key, reviewer, run))?;
        self.book.issue_approval(
            id,
            &ApprovalUse {
                plan: planned.key.clone(),
                reviewer: reviewer.into(),
                consumed_by: run.into(),
            },
        )?;
        Ok(key)
    }
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub fn apply<P: EstateProvider, G: EffectGate, C: Clock>(
        &self,
        provider: &mut P,
        gate: &mut G,
        clock: &C,
        planned: &Planned,
        approval_id: &str,
        approval_key: Key,
        run: &str,
    ) -> Result<ApplyOutcome, Error> {
        if clock.now() > planned.plan.expires_at {
            return Err(Error::Refused {
                reason: "expired plan".into(),
                observed: clock.now().to_string(),
            });
        }
        let (current_card, current_inventory) = provider.discover()?;
        if current_card != planned.plan.provider || current_inventory != planned.plan.inventory {
            return Err(Error::Refused {
                reason: "stale discovery".into(),
                observed: format!("revision={}", current_inventory.revision),
            });
        }
        self.book.consume_approval(approval_id, run)?;
        let mut held = vec![];
        for target in &planned.plan.targets {
            let name = target.as_str();
            let observed = self.book.lease_state(name)?;
            if observed.is_some() {
                return Err(Error::Refused {
                    reason: "target unavailable".into(),
                    observed: format!("{observed:?}"),
                });
            }
            let lease = LeaseRecord {
                holder: run.into(),
                generation: 1,
                expires_at: clock.now() + planned.plan.policy.ttl,
                quarantined: false,
            };
            self.book.lease(name, None, &lease)?;
            held.push((name.to_owned(), lease));
        }
        self.book.append(
            run,
            EventKind::Opened {
                plan: planned.key.clone(),
            },
        )?;
        if let Some(p) = &planned.preview {
            let k = self.book.intern(p)?;
            self.book.append(run, EventKind::Preview { evidence: k })?;
        }
        self.book.append(run, EventKind::DispatchIntent)?;
        let inventory_key = self.book.intern(&planned.plan.inventory)?;
        let request = EffectRequest {
            plan: planned.key.clone(),
            provider: planned.plan.provider.provider.clone(),
            operation: planned.plan.operation.exposure.clone(),
            project: planned.plan.project.clone(),
            inventory: inventory_key,
            targets: planned.plan.targets.clone(),
            approval: approval_key,
        };
        let provider_plan = to_provider_plan(&planned.key, &planned.plan)?;
        let approval = Approval {
            version: 1,
            plan: provider_plan.id.clone(),
            project: planned.plan.project.clone(),
            granted: true,
        };
        let dispatched = gate.perform(&request, || provider.perform(&provider_plan, &approval));
        let run_record = match dispatched {
            Ok(r) => {
                self.book.append(
                    run,
                    EventKind::ProcessAttempt {
                        dispatched: true,
                        detail: "returned".into(),
                    },
                )?;
                r
            }
            Err(Error::Estate(EstateError::UnknownAfterDispatch)) => {
                self.quarantine(run, &mut held, "unknown-after-dispatch")?;
                return Ok(ApplyOutcome {
                    run: run.into(),
                    state: ApplyState::Quarantined,
                    verification: None,
                });
            }
            Err(e) => {
                self.release(run, &mut held)?;
                return Err(e);
            }
        };
        for e in gate.perform(&request, || provider.events(&run_record, 4096))? {
            self.book.append(
                run,
                EventKind::Callback {
                    sequence: u64::from(e.sequence),
                    kind: format!("{:?}", e.kind),
                },
            )?;
        }
        let verification = if planned.plan.policy.verify {
            match gate.perform(&request, || provider.verify(&run_record)) {
                Ok(v) => {
                    self.book
                        .append(run, EventKind::Verification { passed: v.passed })?;
                    if !v.passed {
                        self.quarantine(run, &mut held, "verification-failed")?;
                        return Err(EstateError::VerificationFailed.into());
                    }
                    Some(v)
                }
                Err(e) => {
                    self.quarantine(run, &mut held, "verification-failed")?;
                    return Err(e);
                }
            }
        } else {
            None
        };
        self.release(run, &mut held)?;
        let state = if verification.is_some() {
            ApplyState::Verified
        } else {
            ApplyState::ProcessCompleteUnverified
        };
        self.book.append(
            run,
            EventKind::Final {
                state: match state {
                    ApplyState::Verified => "verified",
                    _ => "process-complete/unverified",
                }
                .into(),
            },
        )?;
        Ok(ApplyOutcome {
            run: run.into(),
            state,
            verification,
        })
    }
    fn release(&self, run: &str, held: &mut Vec<(String, LeaseRecord)>) -> Result<(), Error> {
        for (target, lease) in held.drain(..).rev() {
            let free = LeaseRecord {
                holder: String::new(),
                generation: lease.generation + 1,
                expires_at: 0,
                quarantined: false,
            };
            self.book.lease(&target, Some(&lease), &free)?;
            self.book.append(
                run,
                EventKind::Cleanup {
                    target,
                    generation: lease.generation,
                },
            )?;
        }
        Ok(())
    }
    fn quarantine(
        &self,
        run: &str,
        held: &mut Vec<(String, LeaseRecord)>,
        reason: &str,
    ) -> Result<(), Error> {
        for (target, lease) in held.drain(..).rev() {
            let q = LeaseRecord {
                holder: run.into(),
                generation: lease.generation + 1,
                expires_at: lease.expires_at,
                quarantined: true,
            };
            self.book.lease(&target, Some(&lease), &q)?;
            self.book.append(
                run,
                EventKind::Quarantined {
                    target,
                    reason: reason.into(),
                },
            )?;
        }
        self.book.append(
            run,
            EventKind::Final {
                state: "quarantined".into(),
            },
        )?;
        Ok(())
    }
    pub fn renew<C: Clock>(
        &self,
        run: &str,
        targets: &[Symbol],
        clock: &C,
        ttl: u64,
    ) -> Result<(), Error> {
        for t in targets {
            let old = self
                .book
                .lease_state(t.as_str())?
                .ok_or_else(|| Error::Refused {
                    reason: "lease lost".into(),
                    observed: "absent".into(),
                })?;
            if old.holder != run || old.quarantined {
                return Err(Error::Refused {
                    reason: "lease lost".into(),
                    observed: format!("{old:?}"),
                });
            }
            let new = LeaseRecord {
                expires_at: clock.now() + ttl,
                generation: old.generation + 1,
                ..old.clone()
            };
            self.book.lease(t.as_str(), Some(&old), &new)?;
        }
        Ok(())
    }
    pub fn reconcile<A: ArtifactReader>(
        &self,
        run: &str,
        artifacts: &A,
    ) -> Result<RunState, Error> {
        let projection = self.book.projection(run)?;
        if projection.state != "open" {
            return Ok(parse_state(&projection.state));
        }
        let state = artifacts
            .reconcile(run)?
            .unwrap_or(RunState::UnknownAfterDispatch);
        self.book.append(
            run,
            EventKind::Final {
                state: format!("{state:?}"),
            },
        )?;
        Ok(state)
    }
}
fn to_provider_plan(key: &Key, p: &ImmutablePlan) -> Result<Plan, Error> {
    Ok(Plan {
        version: 1,
        id: Symbol::new(format!("plan/{}", &key.0[7..23]))?,
        operation: p.operation.clone(),
        inventory_revision: p.inventory.revision,
        project: p.project.clone(),
        risk: p.risk.clone(),
        preview: None,
    })
}
fn parse_state(s: &str) -> RunState {
    match s {
        "verified" | "Changed" => RunState::Changed,
        "process-complete/unverified" => RunState::Running,
        "Unchanged" => RunState::Unchanged,
        _ => RunState::UnknownAfterDispatch,
    }
}

#[cfg(test)]
mod controller_acceptance_tests;
#[cfg(test)]
#[allow(clippy::many_single_char_names)]
mod tests;
