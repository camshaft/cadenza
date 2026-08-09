//! In-crate round-trip / projection test suites for the ML front-end.
//!
//! These exercise the crate's own public surface (parser/printer/sexpr/doc_item) plus the re-exported
//! `cadenza_ast` core (ast/canon/codec) — end-to-end over the whole read→print→re-read pipeline. They
//! were originally `tests/*.rs` CARGO INTEGRATION tests (each a SEPARATE test binary = an extra full
//! crate link), but the standing directive is NO integration tests: they carry no subprocess and need
//! no separate binary, so they live here as ordinary `#[cfg(test)]` in-crate units (compiled with the
//! lib, linking nothing extra, and reachable via `crate::` internal paths). Coverage is unchanged from
//! the integration form.
//!
//! `corpus_roundtrip` reads `spec/semantics/*.sexp` relative to `CARGO_MANIFEST_DIR` (the crate root),
//! which is identical whether the test lives under `tests/` or `src/` — so the fixture path is stable.

mod canon_cross_surface;
mod corpus_roundtrip;
mod doc_item_projection;
mod generative_roundtrip;
