//! cdz-agent-host — the REAL effect executors + authorizer that let a `cdz-kernel` session run against
//! the world.
//!
//! The kernel (`cdz-kernel`) is a generic if-this-then-that spine: it folds a reducer over events,
//! authorizes each requested effect, durably dispatches it, and folds the result back. It defines the
//! [`cdz_kernel::executor::Executor`] and [`cdz_kernel::authz::Authorize`] TRAITS but ships only
//! test/interim impls (a `RecordingExecutor`, the `live-exec` `ShellExecutor`, a flat capability
//! `Authorizer`). The ONE thing missing to make an agent LOOP end-to-end is a set of REAL executors:
//! when a reducer's `Model` effect actually calls Bedrock and folds the completion back, an agent runs.
//!
//! This crate provides those executors, layered so the default build stays hermetic (no network, no
//! credentials — the same discipline `cdz-kernel`'s `live-exec` feature uses):
//! - [`ClockExecutor`] — `Now` → the system wall clock. Hermetic (no feature gate): it needs no network
//!   and no credentials, and its result is RECORDED in the log, so replay reuses the frozen instant
//!   (§9c reads-are-effects — the reducer never reads the clock directly; determinism lives in the log,
//!   not the executor).
//! - [`ModelExecutor`] — `Model` → a model completion, the headline: when a reducer's `Model` effect
//!   reaches a model and the completion folds back, an agent loops end-to-end. It is GENERIC over a
//!   [`ModelTransport`] so its effect-mapping logic is hermetically testable; the real Bedrock transport
//!   (SigV4 + the cred-broker) lands behind `live-net`, a stub drives the default gate.
//! - [`HttpExecutor`] — `Http` → an HTTP response, for an agent that fetches a URL. Same transport-seam
//!   shape as the model executor (generic over an [`HttpTransport`]); the real client lands behind
//!   `live-net`, a stub drives the default gate. The URL's host is gated by the kernel's SEC-F1 `HostIn`
//!   capability (the SSRF/exfil guard) before dispatch, so this executor does not re-authorize.
//!
//! All executor errors are RECOVERABLE + classified for the kernel's supervision tree (see [`retry`]):
//! an `EffectOutcome::Err` reason leads with a `RETRYABLE:`/`PERMANENT:` token so a supervisor decides
//! backoff-retry vs give-up — never a panic, never a silent drop (the operator's error-resilience floor).
//!
//! The shared surface with `cdz-kernel` is ONLY the trait signatures; this crate never edits kernel src.

pub mod clock;
pub mod http;
pub mod model;
pub mod retry;

pub use clock::ClockExecutor;
pub use http::{HttpExecutor, HttpTransport};
pub use model::{ModelExecutor, ModelTransport};
pub use retry::{classify, permanent, retryable, Retryability};
