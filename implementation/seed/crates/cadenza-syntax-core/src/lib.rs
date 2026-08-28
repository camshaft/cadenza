//! `cadenza-syntax-core` — surface-agnostic shared bottom of the syntax front-end.
//!
//! The pieces every surface reader/printer needs but that are NOT themselves a syntax:
//! - [`span`]/[`spans`] — a byte range and the `StructId → Span` table (source-location infra).
//! - [`arena_read`] — the shared arena read-helpers (`list_items`/`child_tail`/`str_leaf`/…) the
//!   surface readers project the arena through.
//!
//! Depends only on `cadenza-ast`. Re-exports `cadenza_ast::ast` as `crate::ast` so the moved modules'
//! `crate::ast::…` paths keep resolving unchanged.

/// Re-export so the moved modules' `crate::ast::{…}` paths resolve in this crate.
pub use cadenza_ast::ast;

pub mod arena_read;
pub mod span;
pub mod spans;
