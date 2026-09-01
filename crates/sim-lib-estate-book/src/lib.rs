#![forbid(unsafe_code)]
//! Immutable, hash-linked estate history over a minimal Table/Dir CAS contract.
// conformance: estate history is immutable, linked, and rebuilt only from durable records.

mod book;

pub use book::*;
