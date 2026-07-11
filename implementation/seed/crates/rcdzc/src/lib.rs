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

// The Hindley-Milner core: substitution, unification (occurs-check), scheme instantiation. The
// machinery the one generic application rule uses; pure over `Ty`.
pub mod unify;

// The compile-time evaluator: the ONE application reduction through `Meta.apply` — projecting a head
// value's meta channel, applying a native primitive (an arithmetic op or a type constructor), and
// building a value (e.g. `(Int 64)` → a width-64 module) by appending arena nodes.
pub mod eval;

// The per-node rung forms (each an entry of a column keyed by AST `StructId`).
pub mod core;
pub mod resolved;

// The prelude — the one map of built-in bindings, installed as ordinary AST records at load. A
// built-in module is just a record; nothing is privileged by name or by shape.
pub mod prelude;

// The query engine: the single `Db` is PURE DATA (the AST + the columns); each query is a free
// function in its own module over `&mut Db`, and each module owns exactly one column's fills —
// `resolve` fills `resolved`, `infer` fills `types`, `lower` fills `core`. A query reads another
// module's fact by calling that module's producer (which fills it lazily), never a raw column.
pub mod db;
pub mod infer;
pub mod lower;
pub mod resolve;

// The target-neutral boundary layout (exports by declared signature, reachable set, emission order),
// computed once above the backend seam.
pub mod layout;

// The backend seam + the wasm backend. Everything above the seam is target-neutral; a backend is a
// function of the typed core and the layout, chosen at the seam by `Target`.
pub mod backend;

// The build-tool ABI (kinded artifacts + diagnostics) and the pure compilation entry (no I/O — the
// part that ports to the Cadenza self-host). A CLI bin puts filesystem/args on top of `compile`.
pub mod abi;
pub mod compile;

pub use abi::{Artifact, CompileOutput, Diagnostic, Severity};
pub use backend::Target;
pub use compile::{compile, compile_component};

// Shared test fixtures + the Stage-0 end-to-end tests (compiled only under `#[cfg(test)]`).
#[cfg(test)]
mod testkit;
#[cfg(test)]
mod tests;
