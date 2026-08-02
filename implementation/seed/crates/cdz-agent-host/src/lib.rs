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
//! - a Bedrock `Model` executor + a real `Http` client land behind the `live-net` feature (they need
//!   egress / the operator's Bedrock cred-broker), so a CI runner without egress still gates the crate.
//!
//! The shared surface with `cdz-kernel` is ONLY the trait signatures; this crate never edits kernel src.

pub mod clock;

pub use clock::ClockExecutor;
