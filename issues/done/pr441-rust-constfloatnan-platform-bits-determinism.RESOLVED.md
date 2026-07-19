# PR review comments — mirrored from GitHub PR #441 (Copilot inline)

- **PR:** #441 (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/backend/rust/expr.rs:794` (+ test rust/tests.rs:2343)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3592691445, 3592691454
- **Links:** https://github.com/camshaft/cadenza/pull/441#discussion_r3592691445 , #discussion_r3592691454

## Comments (verbatim)
> Emitting `ConstFloatNan` as `f32::NAN`/`f64::NAN` relies on the platform's chosen NaN payload/bit pattern. If the rest of the system treats NaN values by canonical byte form (as the wasm/runtime float-eq work does), a platform-specific NaN bit pattern could disagree with the canonical form.
> [test] This assertion hard-codes the exact Rust spelling `f64::NAN`. If codegen switches to emitting canonical `from_bits(...)` for determinism, update the test to match.

## Liaison triage — CONFIRMED against trunk
Confirmed: the rust backend emits `ConstFloatNan` as the literal string `"f32::NAN"` / `"f64::NAN"`
(expr.rs:794-797). Those use the platform's chosen NaN bit pattern. The fleet's float-eq work
(runtime + wasm) canonicalizes NaN by BYTE form (the `scalar-float-eq` / NaN-canonicalizing compare) —
so a platform-specific `f64::NAN` payload emitted by the rust backend could DISAGREE with the canonical
NaN the rest of the system compares against (a cross-backend determinism/consistency risk, not
necessarily a live bug today). FIX: emit a canonical `f64::from_bits(<canonical NaN>)` (matching the
runtime's canonical NaN) for cross-backend determinism, and update the rust/tests.rs:2343 assertion to
match. v-rust-backend. Fix on `trunk`. Quotes + links in queue file.

<!-- UPDATE 2026-07-16 (corpus-bugfix): v-rust-backend investigated — NO LIVE disagreement (Rust f64::NAN bits = 0x7FF8… = codebase CANON_NAN_BITS, same as wasm select.rs:5443 + FloatCompare folds every NaN to it). Hardening (explicit from_bits + test update) QUEUED as its next slice after its pending ValueEq MR a87fd2808. Not a live bug; self-documenting hardening in flight. -->
