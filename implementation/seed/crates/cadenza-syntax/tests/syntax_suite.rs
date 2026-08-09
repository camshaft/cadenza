//! Single integration-test binary aggregating `cadenza-syntax`'s external-consumer suites.
//!
//! Each suite below was its OWN `tests/*.rs` file, which Cargo compiles as a SEPARATE test binary —
//! four full links of the crate + four codegen cycles per `cargo test`. They are pure public-API tests
//! (no subprocess, no shared `common/` state), so nothing about them needs a separate binary; the split
//! only multiplied link time. Consolidating them here as `mod`s of files under `tests/suite/` (a SUBDIR,
//! which Cargo does NOT auto-compile as its own binary) collapses the four links into one while keeping
//! every test function, its module path, and its external-integration semantics byte-identical.
//!
//! To add another external-consumer suite: drop the file in `tests/suite/` and `mod` it here — do NOT
//! create a new top-level `tests/*.rs` (that re-introduces a separate binary).

mod suite {
    mod canon_cross_surface;
    mod corpus_roundtrip;
    mod doc_item_projection;
    mod generative_roundtrip;
}
