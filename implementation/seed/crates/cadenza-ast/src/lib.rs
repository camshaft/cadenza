//! `cadenza-ast` — the AST value model and its canonical binary form, as a dependency-light bottom
//! crate shared by every consumer of the canonical wire format.
//!
//! The canonical form of a Cadenza program is a stable binary serialization of its abstract syntax
//! tree (`constitution.md` "Programs Are Readable By Agents And Humans";
//! `spec/contracts/ast-encoding.md`). That encoding, its total decoder, and
//! the value model they operate over were originally grown inside `cadenza-syntax`; they live here so
//! `cadenza-syntax` (text front-end), `rcdzc` (compiler), and the agent-harness kernel can all depend
//! on ONE implementation — one encoder/decoder, one version header, one value model — rather than each
//! carrying a parallel copy that drifts from the spec pins.
//!
//! `cadenza-syntax` re-exports what moves here, so its public API stays byte-stable and its corpus
//! round-trip gate keeps passing. This is a REFERENCE implementation destined to be rewritten in
//! Cadenza; the durable artifact is the CONTRACT (the wire format + total decode), not this code.
//!
//! This crate holds the value model ([`ast`] — `Arenas`/`Leaf`/`Struct`/`Builder`), its canonical form
//! ([`canon`]), the binary codec ([`codec`]), the LEB128 varint the codec is built on ([`leb128`]), and
//! the small hasher they share ([`fxhash`]). Surface-dependent tests (which build inputs via
//! `cadenza-syntax`'s text/s-expr readers) live in `cadenza-syntax`'s integration tests, since this
//! crate sits BELOW those surfaces.

// The minimal codec core is `no_std` + `alloc`: `cdz-runtime` (`#![no_std]`, frozen-hash) builds this
// crate with `--no-default-features`. The `std` default feature turns std back on for the full surface.
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod ast;
pub mod codec;
pub mod leb128;

// The full surface — canonicalization and the FxHash maps it uses — is std-only
// (`HashMap`/`num-bigint`/NFC). Gated behind `std`; compiled out of the no_std minimal core.
#[cfg(feature = "std")]
pub mod canon;
#[cfg(feature = "std")]
pub mod fxhash;
