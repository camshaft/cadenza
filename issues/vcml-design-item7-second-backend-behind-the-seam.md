# Design/scoping: item-7 multi-backend — ADD A SECOND BACKEND behind the existing seam

**Scoped:** 2026-08-01, v-compiler-ml. Frontier scan after item-3 (real HM) closed-for-scope + emit-once
slice-3 deferred (blocked on v-cperf's re-lower). Foundation-hardening items 1/2/3 DONE; 4 (memo-MODEL done,
full replacement gated/diminishing); 6 (API parity) + 7 (multi-backend) remain. This doc scopes item-7.

## Current state (verified on trunk d247bf556)
The multi-backend SEAM ALREADY EXISTS and is tested (emit-db.cdz):
- `type Target = Wasm` (with a doc-noted `Rust` extension point).
- `target-emit(target: Target, c: Core)` — the dispatch (rcdzc's `backend::emit(db, target)` analog),
  currently one arm `Target.Wasm => emit-wasm-module(c)`.
- `emit-src-for(target, s)` — SOURCE→artifact through the seam (read→lower to target-NEUTRAL Core→dispatch);
  `emit-src(s)` = the Wasm specialization. `emit-module(c)` routes through `target-emit` for back-compat.
- Tests: `em-seam-target-wasm-emits` (seam == direct wasm arm), `em-seam-src-for-wasm-matches-emit-src`.

So the operator's "multi-backend architecture from the START" is satisfied STRUCTURALLY. What is NOT done:
there is only ONE backend. The seam is unproven until a SECOND `Target` arm exists — adding one is where the
seam either generalizes cleanly or reveals it's secretly wasm-shaped (the real stress test).

## The scope question (for concierge → operator steer)
Which second backend, and is it wanted NOW?
- **Option A — `Target.Rust` (mirror rcdzc backend/rust):** emit Rust SOURCE from Core. Highest-fidelity to
  the rcdzc guide + the operator's real multi-backend intent, but a LARGE emitter (every Core node → Rust
  expr/stmt; needs a Rust-value model, arithmetic, the narrow-width trapping semantics in Rust, a `fn main`
  harness). Multi-tick. Real payoff: a genuinely different target proves the seam + the Core IR is neutral.
- **Option B — `Target.Text` / debug-render (lightweight):** emit a readable TEXTUAL rendering of the Core
  (S-expr-ish `(add (const 1) (const 2))`), OR a trivial "constant folded value" target. SMALL (a Core walk
  to a String), proves the seam generalizes to a non-wasm shape, and is useful as a debug/inspection artifact.
  Lower payoff (not a real codegen target) but a fast, low-risk seam-proof + genuine stress test of the
  Core-is-neutral claim.
- **Option C — DEFER item-7 entirely:** the seam is in place; adding a backend has no consumer demand yet
  (the W4 differential + gate only exercise Wasm). Like the polymorphism call, a second backend may be
  premature machinery. Pivot to item-6 (API parity artifacts-in/out) or return to emit-once when v-cperf's
  re-lower lands.

## Recommendation (v-compiler-ml lean)
Lean B-then-A OR C. A lightweight `Target.Text` debug-render (B) is a cheap, honest seam-proof + a real
stress test (does Core render cleanly to a non-wasm target?) that ALSO gives a useful inspection artifact —
low risk, ships the seam-generalization proof. Full `Target.Rust` (A) is the real multi-backend deliverable
but a large arc worth an explicit operator go (like the generics call). If the operator has no near-term
multi-backend need, C (defer) matches the measure-first ethos — don't build a second backend without a
consumer. AWAIT STEER before building either A or B (both are real work; C needs no work).

## Note
This composes with the PENDING operator generics-roadmap call (item-3 polymorphism) and the DEFERRED
emit-once slice-3 (v-cperf's closedness-re-lower). If the operator prioritizes generics or a Rust backend,
that reorders. Non-blocking; filed for the steer.
