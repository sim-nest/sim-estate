#![forbid(unsafe_code)]
//! One read-only estate directory and the typed calls accepted by its organ.
// conformance: estate projections are read-only and calls remain capability-shaped.

use serde::{Deserialize, Serialize};
use sim_lib_estate::{ImmutablePlan, Planned};
use sim_lib_estate_book::{Book, BookError, EventEnvelope, Key, Projection, Table};

/// Stable rows exposed by the estate directory.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Row {
    Provider {
        id: String,
        revision: String,
    },
    Inventory {
        key: Key,
        provider: String,
    },
    Plan {
        key: Key,
        plan: Box<ImmutablePlan>,
    },
    Approval {
        id: String,
        plan: Key,
        reviewer: String,
    },
    Run {
        id: String,
        plan: Key,
        state: String,
    },
    Event {
        run: String,
        event: EventEnvelope,
    },
    Quarantine {
        run: String,
        target: String,
        reason: String,
    },
    Summary {
        state: String,
        count: u64,
    },
}

/// Read-only projection contract. It deliberately has no mutation method.
pub trait EstateDir {
    fn rows(&self) -> Result<Vec<Row>, BookError>;
    fn run(&self, id: &str) -> Result<Projection, BookError>;
}

/// Rebuildable projection over the organ's immutable book.
pub struct BookDir<T> {
    book: Book<T>,
    runs: Vec<String>,
    catalog: Vec<Row>,
}
impl<T: Table> BookDir<T> {
    pub fn new(table: T, runs: impl IntoIterator<Item = String>) -> Self {
        Self {
            book: Book::new(table),
            runs: runs.into_iter().collect(),
            catalog: Vec::new(),
        }
    }

    /// Add immutable provider, inventory, plan, approval, or quarantine rows.
    #[must_use]
    pub fn with_catalog(mut self, rows: impl IntoIterator<Item = Row>) -> Self {
        self.catalog.extend(rows);
        self
    }
}
impl<T: Table> EstateDir for BookDir<T> {
    fn rows(&self) -> Result<Vec<Row>, BookError> {
        let mut rows = self.catalog.clone();
        let mut states = std::collections::BTreeMap::<String, u64>::new();
        for id in &self.runs {
            let projection = self.book.projection(id)?;
            rows.push(Row::Run {
                id: id.clone(),
                plan: projection.plan.clone(),
                state: projection.state.clone(),
            });
            *states.entry(projection.state.clone()).or_default() += 1;
            rows.extend(projection.events.iter().cloned().map(|event| Row::Event {
                run: id.clone(),
                event,
            }));
        }
        rows.extend(
            states
                .into_iter()
                .map(|(state, count)| Row::Summary { state, count }),
        );
        Ok(rows)
    }
    fn run(&self, id: &str) -> Result<Projection, BookError> {
        self.book.projection(id)
    }
}

/// Exact operations shared by terminal, Surface, SDK, and agent tools.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Call {
    Discover,
    Plan {
        operation: String,
    },
    Preview {
        plan: Key,
    },
    Review {
        plan: Key,
    },
    Apply {
        plan: Key,
        approval: Key,
    },
    Watch {
        run: String,
    },
    Verify {
        run: String,
    },
    Reconcile {
        run: String,
    },
    Override {
        run: String,
        plan: Key,
        approval: Key,
    },
    History {
        run: Option<String>,
    },
    Doctor,
}

/// Capability context for ordinary Shape-dispatched functions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authority {
    Observe,
    AutomatedApply,
    HumanReview,
}

/// A pre-approved plan supplied to an automated caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Preapproved {
    pub planned: Planned,
    pub approval: Key,
}

/// Reject calls that exceed the caller's capability; review issuance is human-only.
pub fn authorize(
    call: &Call,
    authority: Authority,
    preapproved: Option<&Preapproved>,
) -> Result<(), &'static str> {
    match call {
        Call::Discover
        | Call::Preview { .. }
        | Call::Watch { .. }
        | Call::Verify { .. }
        | Call::History { .. }
        | Call::Doctor => Ok(()),
        Call::Apply { plan, approval }
            if authority == Authority::AutomatedApply
                && preapproved
                    .is_some_and(|p| &p.planned.key == plan && &p.approval == approval) =>
        {
            Ok(())
        }
        Call::Review { .. } if authority == Authority::HumanReview => Ok(()),
        Call::Plan { .. } | Call::Reconcile { .. } | Call::Override { .. }
            if authority == Authority::HumanReview =>
        {
            Ok(())
        }
        _ => Err("estate capability refused"),
    }
}

/// Parse the fixed Shape-like call vocabulary; unknown fields and command text fail closed.
pub fn parse_call(json: &str) -> Result<Call, serde_json::Error> {
    serde_json::from_str(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automated_authority_observes_but_cannot_review_or_fabricate_approval() {
        assert!(authorize(&Call::Discover, Authority::Observe, None).is_ok());
        let key = Key("sha256:plan".into());
        assert!(
            authorize(
                &Call::Review { plan: key.clone() },
                Authority::AutomatedApply,
                None
            )
            .is_err()
        );
        assert!(
            authorize(
                &Call::Apply {
                    plan: key,
                    approval: Key("sha256:fake".into())
                },
                Authority::AutomatedApply,
                None
            )
            .is_err()
        );
    }
    #[test]
    fn command_text_is_not_part_of_the_call_shape() {
        assert!(
            parse_call(r#"{"call":"apply","command":"rm","plan":{"0":"x"},"approval":{"0":"y"}}"#)
                .is_err()
        );
    }
}
