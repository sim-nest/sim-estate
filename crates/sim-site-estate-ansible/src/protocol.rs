use crate::{bound, hex};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use sim_estate_core::{EstateError, Event, EventKind, RunState, SCHEMA_VERSION, Symbol};

const MAX_EVENT_FILE_BYTES: usize = 4_194_304;
const MAX_EVENT_LINE_BYTES: usize = 8_192;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEvent {
    schema: u16,
    run: String,
    plan: String,
    sequence: u32,
    host: Option<String>,
    task: Option<String>,
    status: String,
    changed: bool,
    prior_hash: String,
    current_hash: String,
    terminal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedEvents {
    pub events: Vec<Event>,
    pub terminal: bool,
    pub state: RunState,
}

pub fn decode_events(
    input: &[u8],
    run: &Symbol,
    plan: &Symbol,
    require_terminal: bool,
) -> Result<DecodedEvents, EstateError> {
    if input.len() > MAX_EVENT_FILE_BYTES {
        return Err(bound("event-file"));
    }
    let mut prior = "0".repeat(64);
    let mut events = Vec::new();
    let mut terminal = false;
    let mut state = RunState::Running;
    for (index, line) in input
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .enumerate()
    {
        if terminal || line.len() > MAX_EVENT_LINE_BYTES {
            return Err(EstateError::MalformedEvent);
        }
        let wire: WireEvent =
            serde_json::from_slice(line).map_err(|_| EstateError::MalformedEvent)?;
        if wire.schema != SCHEMA_VERSION
            || wire.run != run.as_str()
            || wire.plan != plan.as_str()
            || wire.sequence as usize != index
            || wire.prior_hash != prior
            || wire.host.as_ref().is_some_and(|v| Symbol::new(v).is_err())
            || wire.task.as_ref().is_some_and(|v| Symbol::new(v).is_err())
        {
            return Err(EstateError::MalformedEvent);
        }
        let mut canonical: serde_json::Value =
            serde_json::from_slice(line).map_err(|_| EstateError::MalformedEvent)?;
        canonical["current_hash"] = serde_json::Value::String(String::new());
        let expected = hex(&Sha256::digest(
            serde_json::to_vec(&canonical).map_err(|_| EstateError::MalformedEvent)?,
        ));
        if wire.current_hash != expected {
            return Err(EstateError::MalformedEvent);
        }
        prior = wire.current_hash;
        terminal = wire.terminal;
        let kind = match wire.status.as_str() {
            "accepted" => EventKind::Accepted,
            "progress" => {
                if wire.changed {
                    EventKind::Changed
                } else {
                    EventKind::Progress
                }
            }
            "ok" => {
                if wire.changed {
                    EventKind::Changed
                } else {
                    EventKind::Unchanged
                }
            }
            "failed" => {
                state = RunState::Failed;
                EventKind::Progress
            }
            "cancelled" => {
                state = RunState::Cancelled;
                EventKind::Cancelled
            }
            "final" => {
                state = if wire.changed {
                    RunState::Changed
                } else {
                    RunState::Unchanged
                };
                EventKind::Progress
            }
            _ => return Err(EstateError::MalformedEvent),
        };
        events.push(Event {
            version: SCHEMA_VERSION,
            sequence: wire.sequence,
            at: u64::from(wire.sequence),
            kind,
        });
    }
    if require_terminal && !terminal {
        return Err(EstateError::UnknownAfterDispatch);
    }
    Ok(DecodedEvents {
        events,
        terminal,
        state,
    })
}
