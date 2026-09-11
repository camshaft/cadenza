//! The canonical binary-AST value-form toolkit — MOVED to the shared `cadenza-value` crate (2026-09-11) so
//! ONE codec serves every crate that builds/reads canonical Cadenza values (the http control-plane frames
//! here AND a reducer build-guest emitting its contract's response as a typed value), with no drifting copies.
//! Re-exported so every existing `cdz_http_protocol::value::*` user is unaffected.
pub use cadenza_value::*;
