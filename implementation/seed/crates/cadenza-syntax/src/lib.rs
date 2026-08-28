//! `cadenza-syntax` — the decoupled ML front-end for Cadenza.
//!
//! Owns all front-end tooling over a program's two-arena AST ([`ast::Arenas`]): the keyword-based
//! ML text surface (read/print), the binary codec, and source spans. Completely standalone — it
//! depends on no compiler crate.
//!
//! This is a REFERENCE implementation destined to be rewritten in Cadenza; the durable artifacts
//! are the contracts (AST shape, binary bytes, text grammar, span table, total decode), not this
//! code.

// The AST value model, its canonical form, the binary codec, the LEB128 varint, and their shared
// hasher moved to the `cadenza-ast` bottom crate (so rcdzc + the agent-harness kernel co-depend on the
// one wire format). Re-exported here so the public paths (`cadenza_syntax::ast`/`::canon`/`::codec`/
// `::fxhash`/`::leb128`) and the internal `crate::` paths stay byte-stable. `canon`/`codec`'s
// SURFACE-dependent tests (which build inputs via the readers below) relocated to `tests/`.
pub use cadenza_ast::{ast, canon, codec, fxhash};
/// The Cedar surface — split into the `cadenza-syntax-cedar` crate (which owns the heavy `cedar-policy`
/// tree), re-exported here as `cadenza_syntax::cedar` behind the `cedar` feature so callers + the Format
/// driver are unchanged. A `(cedar-policyset (cedar-policy …)…)` document (mirroring Cedar's `pst`) is a
/// projection of the same arena the code surfaces use — data, not a program; no authorization engine.
#[cfg(feature = "cedar")]
pub use cadenza_syntax_cedar as cedar;
/// Shared arena read-helpers (`list_items`/`child_tail`/`str_leaf`/`int_leaf`/`bool_leaf`) the surface
/// readers project the arena through — moved to `cadenza-syntax-core`, re-exported so `crate::arena_read`
/// (and every surface's use of it) stays byte-stable.
pub(crate) use cadenza_syntax_core::arena_read;
/// The `cdz-syntax` command surface, factored into the library so both the standalone `cdz-syntax`
/// bin and the unified `cdz` bin drive one implementation.
#[cfg(feature = "cli")]
pub mod cli;
pub mod convert;
pub mod debug;
/// The Wadler-style pretty-printer engine — moved to `cadenza-syntax-core`, re-exported so
/// `cadenza_syntax::doc` + internal `crate::doc` (every printer) are unchanged.
pub use cadenza_syntax_core::doc;
/// The doc-item projection: fold a program's public surface into a derived `doc-module` doc-AST
/// (`cadenza doc`, design/DESIGN-cadenza-docs.md I1). Structural/syntactic — no typecheck dependency.
pub mod doc_item;
pub mod extern_name;
pub use cadenza_ast::leb128;
/// Generally-useful iterator helpers (span-carrying `Chars`, `Peek2` lookahead) — moved to
/// `cadenza-syntax-core`, re-exported so `cadenza_syntax::iter` + internal `crate::iter` are unchanged.
pub use cadenza_syntax_core::iter;
/// The JSON surface — split into the `cadenza-syntax-json` crate (hand-rolled, no external parser dep),
/// re-exported as `cadenza_syntax::json`. A faithful data document (`(json-object …)`/`(json-array …)`/
/// `(json-null)` + bare scalar leaves) preserving everything real JSON has that the typed value universe
/// would reject or normalize — duplicate & non-identifier keys, key order, heterogeneous arrays, exact
/// numbers, and `null`.
pub use cadenza_syntax_json as json;
pub mod lexer;
/// Shared literal lexing/escapes — moved to `cadenza-syntax-core`, re-exported so `cadenza_syntax::literal`
/// + internal `crate::literal` (every reader + the lexer) are unchanged.
pub use cadenza_syntax_core::literal;
/// The markdown surface: a literate document (`(document …)`) is a projection of the same arena the
/// code surfaces use, and an embedded `cdz`/`ml`/`sexp` code block carries its program as a real
/// arena SUBTREE — homoiconic markdown.
pub mod markdown;
/// The `match`→`let` normalization codemod (opt-in, NOT `cdz fmt`): lowers a single-clause
/// irrefutable-unguarded `match` to the equivalent `let`.
pub mod match_to_let;
pub mod parser;
pub mod printer;
pub mod query;
/// Shared REPL module-assembly: assemble "a buffer of definitions + one expression" into one runnable
/// program (`(do item… (def (cdz-repl-eval) <expr>) (export cdz-repl-eval))`). The surface half of a
/// calculator/REPL, reused by every surface (`cdz-wasm::repl_eval`, the native `cdz calc`) so they
/// never drift in how the program is built.
pub mod repl;
pub mod sexpr;
/// Source spans + the `StructId → Span` table — moved to `cadenza-syntax-core`, re-exported so
/// `cadenza_syntax::span`/`::spans` (public API) + internal `crate::span`/`crate::spans` are unchanged.
pub use cadenza_syntax_core::{span, spans};
pub mod token;
/// The TOML surface — split into the `cadenza-syntax-toml` crate (which owns `toml_edit`), re-exported
/// as `cadenza_syntax::toml_surface` (name kept so callers are unchanged). A source-faithful config
/// document (`(toml-document …)` — comments, whitespace, raw scalar spellings as `Str`-leaf "decor"
/// nodes) is a projection of the same arena the code surfaces use; byte-exact round-trip when unmutated.
pub use cadenza_syntax_toml as toml_surface;

pub use ast::{Arenas, Builder, Decimal, Leaf, LeafId, Radix, Struct, StructId};

// In-crate round-trip / projection test suites (relocated from `tests/*.rs` integration binaries per the
// no-integration-tests directive — same coverage, compiled with the lib, no separate binary).
#[cfg(test)]
mod roundtrip_tests;
// Surface tests relocated up from the split-out `cadenza-syntax-core` / `cadenza-syntax-cedar` bottom
// crates (they need a surface reader + `canon`, or the ML-printer fallback, which those below-the-surface
// crates may not depend on). In-crate per the no-integration-tests house style.
#[cfg(test)]
mod surface_tests;
