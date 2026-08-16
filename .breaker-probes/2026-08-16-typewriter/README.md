# tpw ladder — "let-binding reference has no local slot" NEW FACE (2026-08-16, tick 1604)

The adv-20 error string resurfaces with an unrelated recipe (adv-20 was the
let-bound Bytes.from-bytes decode-match face, closed). Uniform decline on
wasm + rust + rust-async (gate todo ×3), error from
implementation/seed/crates/rcdzc/src/backend/wasm/select.rs.

## Minimal recipe (tpwJ-min-repro / tpwJ-case) — ALL ingredients required
A handler arm whose let binder holds a NESTED if:
- OUTER condition reads a STATE tuple field (`(% (+ k 1) 3)`)
- INNER condition reads the def parameter (`(% n 3)`)
- binder consumed ≥3 times (condition + both branch answer/rebuild)
- ≥3 dispatches of that op
- plus a SECOND op that also reads state fields

## Ladder (each drops one ingredient → compiles)
- tpwN: single k-keyed if (no nesting) → 100 B, passes
- tpwP variant: inner condition constant → passes
- tpwQ: binder consumed only 2× → 918 B, passes
- tpwE: second op answers a constant (no state read) → passes
- 2 dispatches instead of 3 → passes
- x-keyed outer condition (arg, not state) → 2.6 KB, passes
- single-if n-keyed 4-tuple (tpw-minH) → 20 KB VALID wasm, passes

Fence interaction: this is NOT the scratch-locals budget (different error),
and binder-consumer count ≤2 rescues it (tnk-consistent) — but kgt0/rlyC
fences are about binder-over-if/perform SCRUTINEES; this is a binder-over-if
VALUE whose outer condition is state-keyed. New axis.

tpw1 (the original 5-dispatch typewriter probe, banked here) hits the same
error. Held from corpus until triaged.
