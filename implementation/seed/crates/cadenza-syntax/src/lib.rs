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
pub mod codec;
pub mod convert;
pub mod doc;
pub mod iter;
pub mod leb128;
pub mod lexer;
pub mod literal;
pub mod parser;
pub mod printer;
pub mod sexpr;
pub mod span;
pub mod spans;
pub mod token;

pub use ast::{Arenas, Builder, Decimal, Leaf, LeafId, Radix, Struct, StructId};
