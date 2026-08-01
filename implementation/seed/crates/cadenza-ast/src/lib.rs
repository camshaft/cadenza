//! `cadenza-ast` — the AST value model and its canonical binary form, as a dependency-light bottom
//! crate shared by every consumer of the canonical wire format.
//!
//! The canonical form of a Cadenza program is a stable binary serialization of its abstract syntax
//! tree (`constitution.md` §X; `spec/contracts/ast-encoding.md`). That encoding, its total decoder, and
//! the value model they operate over were originally grown inside `cadenza-syntax`; they live here so
//! `cadenza-syntax` (text front-end), `rcdzc` (compiler), and the agent-harness kernel can all depend
//! on ONE implementation — one encoder/decoder, one version header, one value model — rather than each
//! carrying a parallel copy that drifts from the spec pins.
//!
//! `cadenza-syntax` re-exports what moves here, so its public API stays byte-stable and its corpus
//! round-trip gate keeps passing. This is a REFERENCE implementation destined to be rewritten in
//! Cadenza; the durable artifact is the CONTRACT (the wire format + total decode), not this code.
//!
//! This first extraction slice contains only the dependency-free [`leb128`] varint — the foundation the
//! binary codec is built on. The value types (`ast`) and the codec itself land in later slices.

pub mod leb128;
