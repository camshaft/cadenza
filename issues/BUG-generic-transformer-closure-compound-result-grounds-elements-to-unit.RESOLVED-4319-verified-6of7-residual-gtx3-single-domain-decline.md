# BUG (rust-visible inference miscompile): a recursive-generic transformer whose closure result is a COMPOUND grounds the result element types to Unit

**Status:** OPEN — routed to `v-inference` (infer/unify/resolve). Filed per the operator's "no silent miscompiles — commit a tracked repro" directive (a known-open miscompile must be tracked, not silently parked). v-inference is currently spun down; the reactivate-vs-leave-tracked call is with the operator (surfaced by concierge). **The rust backend is EXONERATED** — no rust-backend action.

**Found by:** corpus-mig-1 (migrating an rcdzc closure test to the corpus). **Root-caused + routed by:** v-rust-backend.

**Symptom:** wasm compiles + runs (value `4`); the RUST / rust-async backend fails to build with `error[E0308]: mismatched types`. `cdz check` PASSES.

## Repro

```
(module m
  (type GIter (Nil) (Cons a (GIter a)))
  (def (from-list xs) (match xs ((list) (GIter.Nil)) ((list h .. t) (GIter.Cons h (from-list t)))))
  (def (count it) (match it ((GIter.Nil) 0) ((GIter.Cons _ rest) (+ 1 (count rest)))))
  (def (gmap it f) (match it ((GIter.Nil) (GIter.Nil)) ((GIter.Cons h rest) (GIter.Cons (f h) (gmap rest f)))))
  (def (main) (+ (count (gmap (from-list (list 1 2)) (fn (x) (tuple x x))))
                 (count (gmap (from-list (list "a" "b")) (fn (s) (String.concat s s))))))
  (export main))
```

(NOTE: corpus-mig-1's original snippet wrote `(list a b)` with bare `a`/`b`, which are unbound names — use string literals `"a"`/`"b"`, as above.)

- `cdz check` → passes.
- `cdz compile --target wasm` → runs, value `4` (correct: two 2-element iterators, `count` = 2 each).
- `cdz compile --target rust` (and `rust-async`) → emits source, but `rustc` fails with E0308.

## Root cause (verified via the emitted rust)

The distinguishing feature: a recursive-generic transformer `gmap` threaded with a closure whose **result is an AGGREGATE** (a tuple), instantiated at TWO distinct domains (`Int64 -> (tuple …)` and `String -> String`) in one program, and consumed by `count` which **DISCARDS the element** (`_`), so `count`'s type variable is unconstrained by its own body.

- `gmap` specializes **correctly**: `gmap_mono6(it: GIter<i64>, f: Rc<dyn Fn(i64) -> (i64, i64)>) -> GIter<(i64, i64)>` (its `// cdz-return` note is `(GIter (Tuple Int64 Int64))`).
- But `count` specializes taking `GIter<((), ())>` — the tuple ARITY is right, but the element types are **ERASED TO UNIT**.
- So `count_mono4(gmap_mono6(...))` hands a `GIter<(i64,i64)>` into a fn expecting `GIter<((),())>` → **E0308**.

The bad type originates in `type_of` on the **outer `gmap`-call node** (which is `count`'s argument): it returns `GIter (Tuple Unit Unit)`. Inference grounds the closure-result tuple element type **variables to Unit** instead of `i64`, even though the same information is present (gmap's own specialization resolved them to `i64`). Because `count` discards its element, its type var `a` is pinned only by unifying `GIter a` with the gmap-call result type; if those element vars are still free at that point they ground to Unit.

## Why it is a rust-visible miscompile (not a clean decline)

- `lower::type_specialize` (lower.rs:13882) sets the value-arg type = `type_of(a)` and only declines when the type `has_free_var()`. **Unit is CONCRETE, not free**, so it accepts the wrong type; the rust backend then emits `GIter<((),())>` verbatim.
- Tagless wasm is uniform (a boxed value is a boxed value regardless of element type), so the wrong element type is **invisible** there (runs `4`).
- If inference instead LEFT those elements as free vars, `type_specialize` would cleanly decline (CDZ0201). But the correct fix is to **RESOLVE them to `i64`** (propagate the closure-result element types to the outer transformer-call result) — the program is genuinely well-typed and monomorphizable (wasm proves value `4`), so declining would be an over-reject.

## Fix location (v-inference)

The transformer-closure result-element tie in infer/unify/resolve — the same place that pins `gmap`'s `f` at each instantiation. The OUTER call's result type needs the closure-result element types the callee already resolved, so `type_of` on the transformer-call node returns `GIter (Tuple Int64 Int64)`, not `GIter (Tuple Unit Unit)`.

## Meanwhile

corpus-mig-1 keeps the rcdzc test `a_generic_transformer_maps_a_closure_to_an_aggregate_result_at_two_distinct_domains` **wasm-only** (uses cdz_run/wasm) — no corpus regression. It cannot migrate to a 3-backend corpus run case until this is fixed. When fixed: verify the rust emit produces `GIter<(i64,i64)>`, then migrate as a 3-backend run case.

## Family map (breaker, 2026-08-27) — sibling cells probed on all three targets

Seven-cell matrix around the repro, each cell `cdz check`-clean and wasm-correct (whole family
carries the recursive-sum known-leak, 16–34 cells — the rsl1 class, separate issue):

| cell | closure result | consumer | domains | rust / rust-async |
|---|---|---|---|---|
| gtx1 (= repro) | tuple | discards | 2 | **E0308 miscompile** |
| gtx2 | tuple | PROJECTS `(. h 0)` | 2 | clean decline (todo) |
| gtx3 | tuple | discards | 1 | clean decline (todo) |
| gtx4 | record | discards | 2 | **E0308 miscompile** |
| gtx5 | `(Option.Some x)` | discards | 2 | **PASS (value 4)** |
| gtx6 | list | discards | 2 | **E0308 miscompile** |
| gtx7 | nested tuple | discards | 2 | **E0308 miscompile** |

Refinements to the root-cause model:
- The miscompile covers every STRUCTURAL aggregate result (tuple, record, list, nested tuple) —
  not tuples specifically.
- A NOMINAL generic-sum result (`Option.Some x`) resolves correctly and passes end-to-end —
  observed behavior; consistent with the element-type tie succeeding through a nominal
  constructor application but not through structural constructors (hypothesis, not verified in
  the inference source).
- Both the projecting-consumer and single-domain variants fall into the CLEAN-DECLINE cell
  (over-conservative per the analysis above — the programs are well-typed; wasm proves 5 / 2).
  A fix that ties the element types should flip gtx2/gtx3 decline→pass alongside un-breaking
  the four E0308 cells.
- rust and rust-async agree cell-for-cell.

gtx2/gtx3/gtx5 are pinned in `spec/semantics/09-functions.sexp` (wasm-only rows — the leak
clauses keep them out of the rust battery). The four E0308 cells are reproducible by dropping
the corresponding `(fn (x) …)` into the repro above.

## RESOLVED (breaker same-hour verification, tick 301, 2026-08-27)

Fix #4319 (tie the closure AGGREGATE result element to the domain at the OUTER call node) +
tests #4348 (record/List/nested-tuple ties). Verified against the full 7-cell matrix on rust AND
rust-async, fresh compiler:

- gtx1 (tuple) / gtx4 (record) / gtx6 (List) / gtx7 (nested tuple): **E0308 → PASS, value 4** ✓
- gtx2 (projecting consumer): **decline → PASS, value 5** ✓
- gtx5 (Option): stays green ✓
- **RESIDUAL: gtx3 (SINGLE domain, discarding consumer) still declines on rust/rust-async** —
  an over-conservative reject, not a miscompile (wasm proves value 2; the program is well-typed).
  With only one instantiation the element vars apparently still ground free at the outer node.
  Low priority; corpus pin gtx3 remains a wasm row, flips whenever the single-domain tie lands.

wasm census unchanged across all 7 (the rsl1-class recursive-sum leaks, separate issue).
