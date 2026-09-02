#![forbid(unsafe_code)]
//! Portable estate records and the provider contract.
// conformance: portable estate records preserve bounded provider lifecycle semantics.

mod records;

pub use records::*;

#[cfg(test)]
mod tests;
