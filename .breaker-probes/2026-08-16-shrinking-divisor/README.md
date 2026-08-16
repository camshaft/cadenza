# div1 — shrinking divisor guarded to zero (2026-08-16, tick 1646)

Attack: the shared quotient `(/ x d)` sits in BOTH the answer and the rebuild
of the taken branch while `d` counts down to the ZERO that would make the
division TRAP — the zero-guard branch absorbs it (state untouched save the
counter). A CSE/hoist that lifts `(/ x d)` ABOVE the `(= d 0)` guard would
introduce a division-by-zero trap on the guarded dispatches. Trap-safety of
shared-compound placement is the pin (adjacent to B2's trap-free exclusion
gate-4 that just landed — this is the SOURCE-level twin of that emit-level
concern).

Differential: starting divisor 2 vs 3 reaches the guard one dispatch earlier
on n=0 (rows [62,91,900,900] saves 2) vs n=10 ([43,42,61,900] saves 1).

Hand model: n=10 → 430420619001401; n=0 → 620919009001502 (mixed base:
4 rows base-1000, read tail base-10000).

Pass ×3 wasm + rust + rust-async on trunk d3086251e.
