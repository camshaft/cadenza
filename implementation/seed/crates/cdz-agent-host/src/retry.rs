//! Retryability classification for executor errors.
//!
//! Retryability is now a FIRST-CLASS typed field on [`cdz_kernel::event::EffectOutcome::Err`] (operator Q2),
//! so a reducer's fold matches STRUCTURALLY on the retryability rather than parsing a `RETRYABLE:`/
//! `PERMANENT:` token out of the message string (the old host convention this replaces). The host only
//! CLASSIFIES an error (a Bedrock throttle → [`Retryability::Retryable`], a malformed request →
//! [`Retryability::Permanent`]); the RETRY POLICY (backoff + re-emit) lives in the reducer.
//!
//! Construct a permanent failure via [`cdz_kernel::event::EffectOutcome::err`] and a retryable one via
//! [`cdz_kernel::event::EffectOutcome::err_retryable`]. The classifiers in [`crate::model`] /
//! [`crate::http`] return an [`EffectOutcome`](cdz_kernel::event::EffectOutcome) already carrying the typed
//! retryability. [`Retryability::Permanent`] is the fail-closed default (never auto-retry an unclassified
//! error forever).

/// Re-export the kernel's typed retryability — the single source of truth. The old crate-local enum + the
/// `RETRYABLE:`/`PERMANENT:` string-token convention (consts, `retryable()`/`permanent()`/`classify()`) are
/// gone: retryability now rides structurally on `EffectOutcome::Err { retryability, .. }`.
pub use cdz_kernel::event::Retryability;
