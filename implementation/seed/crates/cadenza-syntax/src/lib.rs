//! `cadenza-syntax` — the decoupled ML front-end for Cadenza.
//!
//! Owns all front-end tooling over a program's two-arena AST ([`ast::Arenas`]): the keyword-based
//! ML text surface (read/print), the binary codec, and source spans. Completely standalone — it
//! depends on no compiler crate.
//!
//! This is a REFERENCE implementation destined to be rewritten in Cadenza; the durable artifacts
//! are the contracts (AST shape, binary bytes, text grammar, span table, total decode), not this
//! code.

pub mod ast;
pub mod canon;
/// The Cedar surface: an authorization-policy document (`(cedar-policyset (cedar-policy …)…)`,
/// mirroring Cedar's `pst`) is a projection of the same arena the code surfaces use — so an agent can
/// structurally construct/modify a policy with Cadenza's tools. Data, not a program; no authorization
/// engine. Arena-idempotent (comments/formatting not preserved).
pub mod cedar;
/// The `cdz-syntax` command surface, factored into the library so both the standalone `cdz-syntax`
/// bin and the unified `cdz` bin drive one implementation.
pub mod cli;
pub mod codec;
pub mod convert;
pub mod debug;
pub mod doc;
pub mod extern_name;
pub mod fxhash;
pub mod iter;
/// The JSON surface: a faithful data document (`(json-object …)`/`(json-array …)`/`(json-null)` plus
/// bare scalar leaves) is a projection of the same arena the code surfaces use. Unlike a native
/// `record`/`list`, it preserves everything real JSON has that the typed value universe would reject
/// or normalize — duplicate & non-identifier keys, key order, heterogeneous arrays, exact numbers,
/// and `null`.
pub mod json;
pub mod leb128;
pub mod lexer;
pub mod literal;
/// The markdown surface: a literate document (`(document …)`) is a projection of the same arena the
/// code surfaces use, and an embedded `cdz`/`ml`/`sexp` code block carries its program as a real
/// arena SUBTREE — homoiconic markdown.
pub mod markdown;
pub mod parser;
pub mod printer;
pub mod query;
pub mod sexpr;
pub mod span;
pub mod spans;
pub mod token;
/// The TOML surface: a source-faithful config document (`(toml-document …)` — comments, whitespace,
/// and each scalar's raw spelling stored as `Str`-leaf "decor" nodes) is a projection of the same
/// arena the code surfaces use. Byte-exact round-trip for an unmutated doc, and fully rewritable (the
/// arena stays the representation). Named `toml_surface` so `toml_edit::` remains the unambiguous crate
/// path.
pub mod toml_surface;

pub use ast::{Arenas, Builder, Decimal, Leaf, LeafId, Radix, Struct, StructId};
