//! The compile-boundary contract types — the vocabulary the build tool speaks across the compiler
//! seam, shared by the compiler that produces them (`rcdzc`) and the thin front-door that delegates to
//! it (`cdz`). See the crate manifest for the extraction rationale (the compile-boundary analogue of
//! `cadenza-ast`'s AST-wire crate).
//!
//! So far the crate holds the pure-data boundary enums — the requested [`Target`], the [`OptLevel`],
//! and the sidecar [`Request`]/[`Query`] request vocabulary — plus the [`sidecar::encode`]/[`decode`]
//! codec that serializes a request list to/from the canonical binary AST (the one `cadenza-ast` dep),
//! and the source [`spans`] side-table (`SpanData`/`LineStarts` + its codec, a debug compile's kinded
//! input). A later slice moves the kinded `Artifact` list + the `{artifacts, diagnostics}` result. The
//! compiler IMPLEMENTATIONS behind the boundary (query eval over a live `Db`, `compile`, the backends)
//! stay in `rcdzc`.

pub mod abi;
pub mod link_map;
pub mod opt;
pub mod overflow;
pub mod sidecar;
pub mod spans;
pub mod target;

pub use abi::{
    Artifact, CompileOutput, Diagnostic, DiagnosticFix, FixKind, Severity, WRAP_HOLE,
    wrap_prefix_suffix,
};
pub use link_map::{FileSpan, KIND_LINK_MAP, decode_link_map, encode_link_map};
pub use opt::OptLevel;
pub use overflow::{OverflowMode, OverflowSpec};
pub use sidecar::{Query, Request, decode, encode};
pub use target::Target;
