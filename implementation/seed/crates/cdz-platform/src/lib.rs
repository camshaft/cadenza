//! The Cadenza platform runtime (`design/cadenza-platform.md`).
//!
//! A single unified crate, built bottom-up. This first slice is the content-hash foundation
//! (spec section 8): [`Hash`], the sole identity in the system. A contract-id is the hash of a
//! contract declaration; a blob is addressed by the hash of its bytes; the content-addressed store
//! is unpermissioned because the hash *is* the capability. Everything downstream — routing, the
//! registry, the store — addresses by it, so it is the correct first primitive to settle.

mod hash;

pub use hash::{Hash, HashParseError};
