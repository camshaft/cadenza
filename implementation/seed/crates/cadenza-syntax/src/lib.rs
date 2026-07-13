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

pub use ast::{Arenas, Builder, Decimal, Leaf, LeafId, Radix, Struct, StructId};
