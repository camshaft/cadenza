# BUG: cadenza backend emits a USER-declared sum value `(: (Leaf n) T)` without re-emitting the `(type T …)` declaration — non-re-compilable

Found tick 446 (2026-08-28) via the dual-path VALUE oracle probing #4913 (cadenza runtime SumNew emit). ROUTED to v-cadenza-backend. Same acceptance class as the beyond-i64 BigInt.of bug (#4881, fixed): the emitted `.ast` must be self-contained/re-compilable (the `cadenza` target's stated round-trip contract).

## Defect
#4913 lowers a runtime `Core::SumNew` of a USER sum to `(: (<Variant> <payload>) <T>)` (variant_head_ast + type_ast), but the module emit never re-emits the user `(type T …)` DECLARATION. Prelude sums (Option/Result) round-trip because the prelude is ambient; a user sum's re-emitted surface references an unknown type + unbound ctors → CDZ0101 on recompile, so the cadenza hop fails on ANY runtime user-sum value.

## Repro (dual-path)
- `(module m (type T (Leaf Int64) (Node Int64)) (def (main (: n Int64)) (if (> n 0) (Leaf n) (Node (- 0 n)))) (export main))`
  → direct wasm OK (`(: (Leaf 8) T)`); `-t cadenza` EMITS (no decline), recompile of the `.ast` fails:
  `CDZ0101 unknown type 'T' … unbound name 'Leaf' … unbound name 'Node'`.
- Qualified-head twin (variant named `Int`, prelude-colliding, exercises the `(. MyT Int)` spelling): same shape —
  `(module m (type MyT (Int Int64) (Other Int64)) (def (main (: n Int64)) (if (> n 0) ((. MyT Int) n) ((. MyT Other) (- 0 n)))) (export main))`
  also emits then fails recompile on unknown `MyT`. (So the collision-qualified spelling itself is UNVERIFIABLE until decls re-emit — re-check it after the fix.)

## Fix direction
Either re-emit the user `(type …)` declarations the emitted values reference (self-contained surface), or DECLINE a user-sum SumNew until decls are emitted — never emit a surface that cannot re-compile.

## Impact
corpus-cadenza REDS/false-fails on any case whose runtime value is a user-declared sum. breaker did NOT touch compiler code; will author a user-sum cadenza witness (incl. the qualified-head twin) once fixed. Repros: ~/breaker-scratch/2026-08-28-cadenza-sum/{usr,col}.sexp.
