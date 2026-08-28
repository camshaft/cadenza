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
/// The Wadler-style pretty-printer engine (`Doc`/`ibox`/`word`/breaks) every surface PRINTER lays its
/// output out through — surface-agnostic, depends on nothing but std.
pub mod doc;
/// Generally-useful iterator helpers shared across surface readers: a span-carrying char iterator
/// ([`iter::Chars`]/[`iter::Char`]) and a two-token lookahead adapter ([`iter::Peek2`]).
pub mod iter;
/// Shared literal lexing/escapes (string/byte/char/symbol unescape, numeric parsing, NFC, bare-name
/// checks) every surface reader + the lexer use to turn literal text into `Leaf` values.
pub mod literal;
pub mod span;
pub mod spans;
