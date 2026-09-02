#![forbid(unsafe_code)]
//! Ansible as a sealed decoration over the canonical process port.

mod protocol;
pub use protocol::{DecodedEvents, decode_events};

mod ansible;

pub use ansible::*;
