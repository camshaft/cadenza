//! The compile-boundary contract types — the vocabulary the build tool speaks across the compiler
//! seam, shared by the compiler that produces them (`rcdzc`) and the thin front-door that delegates to
//! it (`cdz`). See the crate manifest for the extraction rationale (the compile-boundary analogue of
//! `cadenza-ast`'s AST-wire crate).
//!
//! So far the crate holds the pure-data boundary enums — the requested [`Target`], the [`OptLevel`],
//! and the sidecar [`Request`]/[`Query`] request vocabulary. Later slices move the encode/decode codec
//! for those requests (which takes a `cadenza-ast` dep), the kinded `Artifact` list, and the source
//! `spans` side-table. The compiler IMPLEMENTATIONS behind the boundary (the request codec today,
//! query eval over a live `Db`, `compile`, the backends) stay in `rcdzc`.

pub mod opt;
pub mod sidecar;
pub mod target;

pub use opt::OptLevel;
pub use sidecar::{Query, Request};
pub use target::Target;
