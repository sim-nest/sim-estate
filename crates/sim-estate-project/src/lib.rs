#![forbid(unsafe_code)]
//! Strict, pure exposure compilation.
// conformance: exposure compilation admits only closed shaped operations.

mod project;

pub use project::*;

#[cfg(test)]
mod tests;
