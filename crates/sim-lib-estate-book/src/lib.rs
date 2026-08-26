#![forbid(unsafe_code)]
//! Immutable, hash-linked estate history over a minimal Table/Dir CAS contract.
// conformance: estate history is immutable, linked, and rebuilt only from durable records.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Key(pub String);
impl Key {
    #[must_use]
    pub fn content(bytes: &[u8]) -> Self {
        Self(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum BookError {
    #[error("compare-and-swap conflict at {key:?}")]
    Conflict { key: Key, observed: Option<Vec<u8>> },
    #[error("missing immutable object {0:?}")]
    Missing(Key),
    #[error("corrupt book: {0}")]
    Corrupt(String),
    #[error("storage: {0}")]
    Storage(String),
}

/// The subset of generic Table/Dir required by the book: get, immutable put, CAS, and prefix scan.
pub trait Table: Send + Sync {
    fn get(&self, key: &Key) -> Result<Option<Vec<u8>>, BookError>;
    fn put_absent(&self, key: &Key, value: &[u8]) -> Result<(), BookError>;
    fn compare_and_swap(
        &self,
        key: &Key,
        expected: Option<&[u8]>,
        value: &[u8],
    ) -> Result<(), BookError>;
    fn scan_prefix(&self, prefix: &str) -> Result<Vec<(Key, Vec<u8>)>, BookError>;
}

#[derive(Default)]
pub struct MemoryTable(Mutex<BTreeMap<Key, Vec<u8>>>);
impl Table for MemoryTable {
    fn get(&self, key: &Key) -> Result<Option<Vec<u8>>, BookError> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn put_absent(&self, key: &Key, value: &[u8]) -> Result<(), BookError> {
        let mut m = self.0.lock().unwrap();
        match m.get(key) {
            Some(v) if v == value => Ok(()),
            Some(v) => Err(BookError::Conflict {
                key: key.clone(),
                observed: Some(v.clone()),
            }),
            None => {
                m.insert(key.clone(), value.to_vec());
                Ok(())
            }
        }
    }
    fn compare_and_swap(
        &self,
        key: &Key,
        expected: Option<&[u8]>,
        value: &[u8],
    ) -> Result<(), BookError> {
        let mut m = self.0.lock().unwrap();
        let observed = m.get(key).cloned();
        if observed.as_deref() != expected {
            return Err(BookError::Conflict {
                key: key.clone(),
                observed,
            });
        }
        m.insert(key.clone(), value.to_vec());
        Ok(())
    }
    fn scan_prefix(&self, prefix: &str) -> Result<Vec<(Key, Vec<u8>)>, BookError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.0.starts_with(prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub run: String,
    pub sequence: u64,
    pub previous: Option<Key>,
    pub kind: EventKind,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum EventKind {
    Opened { plan: Key },
    DispatchIntent,
    ProcessAttempt { dispatched: bool, detail: String },
    Callback { sequence: u64, kind: String },
    Preview { evidence: Key },
    Verification { passed: bool },
    Cleanup { target: String, generation: u64 },
    Quarantined { target: String, reason: String },
    Final { state: String },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApprovalUse {
    pub plan: Key,
    pub reviewer: String,
    pub consumed_by: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub holder: String,
    pub generation: u64,
    pub expires_at: u64,
    pub quarantined: bool,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Projection {
    pub run: String,
    pub plan: Key,
    pub state: String,
    pub events: Vec<EventEnvelope>,
}

pub struct Book<T> {
    table: T,
}
impl<T: Table> Book<T> {
    #[must_use]
    pub fn new(table: T) -> Self {
        Self { table }
    }
    pub fn intern<V: Serialize>(&self, value: &V) -> Result<Key, BookError> {
        let bytes = serde_json::to_vec(value).map_err(json)?;
        let key = Key::content(&bytes);
        self.table
            .put_absent(&Key(format!("object/{}", key.0)), &bytes)?;
        Ok(key)
    }
    pub fn read<V: DeserializeOwned>(&self, key: &Key) -> Result<V, BookError> {
        let bytes = self
            .table
            .get(&Key(format!("object/{}", key.0)))?
            .ok_or_else(|| BookError::Missing(key.clone()))?;
        serde_json::from_slice(&bytes).map_err(json)
    }
    pub fn append(&self, run: &str, kind: EventKind) -> Result<Key, BookError> {
        let head = Key(format!("run/{run}/head"));
        let observed = self.table.get(&head)?;
        let previous = observed.as_deref().map(decode_key).transpose()?;
        let sequence = match &previous {
            Some(k) => self.read::<EventEnvelope>(k)?.sequence + 1,
            None => 0,
        };
        let event = EventEnvelope {
            run: run.into(),
            sequence,
            previous,
            kind,
        };
        let event_key = self.intern(&event)?;
        let encoded = serde_json::to_vec(&event_key).map_err(json)?;
        self.table
            .compare_and_swap(&head, observed.as_deref(), &encoded)?;
        Ok(event_key)
    }
    pub fn projection(&self, run: &str) -> Result<Projection, BookError> {
        let head = self
            .table
            .get(&Key(format!("run/{run}/head")))?
            .ok_or_else(|| BookError::Corrupt("run has no head".into()))?;
        let mut next = Some(decode_key(&head)?);
        let mut events = vec![];
        while let Some(ref k) = next {
            let event: EventEnvelope = self.read(k)?;
            next.clone_from(&event.previous);
            events.push(event);
        }
        events.reverse();
        for (i, e) in events.iter().enumerate() {
            if e.sequence != i as u64 {
                return Err(BookError::Corrupt("non-contiguous event chain".into()));
            }
        }
        let plan = match &events[0].kind {
            EventKind::Opened { plan } => plan.clone(),
            _ => return Err(BookError::Corrupt("run does not begin with Opened".into())),
        };
        let state = events
            .iter()
            .rev()
            .find_map(|e| {
                if let EventKind::Final { state } = &e.kind {
                    Some(state.clone())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "open".into());
        Ok(Projection {
            run: run.into(),
            plan,
            state,
            events,
        })
    }
    pub fn issue_approval(&self, id: &str, use_: &ApprovalUse) -> Result<(), BookError> {
        let bytes = serde_json::to_vec(use_).map_err(json)?;
        self.table
            .put_absent(&Key(format!("approval/{id}/reviewed")), &bytes)
    }
    pub fn consume_approval(&self, id: &str, run: &str) -> Result<(), BookError> {
        let reviewed = self
            .table
            .get(&Key(format!("approval/{id}/reviewed")))?
            .ok_or_else(|| BookError::Corrupt("approval not reviewed".into()))?;
        let use_: ApprovalUse = serde_json::from_slice(&reviewed).map_err(json)?;
        if use_.consumed_by != run {
            return Err(BookError::Corrupt("approval run binding mismatch".into()));
        }
        self.table.compare_and_swap(
            &Key(format!("approval/{id}/consumed")),
            None,
            run.as_bytes(),
        )
    }
    pub fn lease(
        &self,
        target: &str,
        expected: Option<&LeaseRecord>,
        next: &LeaseRecord,
    ) -> Result<(), BookError> {
        let key = Key(format!("target/{target}/lease"));
        let old = expected.map(serde_json::to_vec).transpose().map_err(json)?;
        let new = serde_json::to_vec(next).map_err(json)?;
        self.table.compare_and_swap(&key, old.as_deref(), &new)
    }
    pub fn lease_state(&self, target: &str) -> Result<Option<LeaseRecord>, BookError> {
        self.table
            .get(&Key(format!("target/{target}/lease")))?
            .map(|v| serde_json::from_slice(&v).map_err(json))
            .transpose()
    }
}
fn decode_key(bytes: &[u8]) -> Result<Key, BookError> {
    serde_json::from_slice(bytes).map_err(json)
}
fn json(e: impl std::fmt::Display) -> BookError {
    BookError::Corrupt(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exercise<T: Table>(book: &Book<T>) {
        let plan = book.intern(&vec!["canonical", "plan"]).unwrap();
        book.append("r", EventKind::Opened { plan: plan.clone() })
            .unwrap();
        book.append("r", EventKind::DispatchIntent).unwrap();
        book.append(
            "r",
            EventKind::Final {
                state: "process-complete/unverified".into(),
            },
        )
        .unwrap();
        let a = book.projection("r").unwrap();
        let b = book.projection("r").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.plan, plan);
    }
    #[test]
    fn projections_rebuild_identically_in_memory() {
        exercise(&Book::new(MemoryTable::default()));
    }
    #[test]
    fn approval_is_single_use() {
        let b = Book::new(MemoryTable::default());
        let u = ApprovalUse {
            plan: Key("p".into()),
            reviewer: "alice".into(),
            consumed_by: "run".into(),
        };
        b.issue_approval("a", &u).unwrap();
        b.consume_approval("a", "run").unwrap();
        assert!(matches!(
            b.consume_approval("a", "run"),
            Err(BookError::Conflict { .. })
        ));
    }
    #[test]
    fn event_head_conflict_preserves_published_event() {
        let t = MemoryTable::default();
        let b = Book::new(t);
        let p = b.intern(&"p").unwrap();
        b.append("r", EventKind::Opened { plan: p }).unwrap();
        let stale = serde_json::to_vec(&Key("sha256:stale".into())).unwrap();
        assert!(matches!(
            b.table
                .compare_and_swap(&Key("run/r/head".into()), Some(&stale), b"x"),
            Err(BookError::Conflict { .. })
        ));
    }
}
