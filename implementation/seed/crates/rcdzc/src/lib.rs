//! `rcdzc` — the reference Cadenza → WebAssembly-component compiler, rebuilt to the reference
//! architecture (`spec/architecture/*.md`). See `Cargo.toml` for the two shaping directives
//! (copy-don't-depend; Cadenza-in-Rust style). This is the Stage-0 skeleton.

// The copied-in syntax foundation (verbatim from `cadenza-syntax`, minus its external deps): the
// two-arena leaf-pool AST, the total binary codec, and the leb128 primitives it rides on.
pub mod ast;
pub mod codec;
pub mod leb128;

// The columns substrate: index-typed arenas + columns, and the diagnostic taxonomy.
pub mod arena;
pub mod diag;

// The solved-type universe (target-neutral).
pub mod ty;

// The per-node rung forms (each an entry of a column keyed by AST `StructId`).
pub mod core;
pub mod resolved;

// The query engine: the single `Db` holding every column, filled lazily by backward demand.
pub mod db;
