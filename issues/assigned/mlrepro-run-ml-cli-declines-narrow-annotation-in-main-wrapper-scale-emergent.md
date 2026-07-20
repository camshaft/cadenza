# `cdz run-ml` CLI DECLINES a narrow annotation `(: v Int8)` inside the `(def (main) …)` module wrapper — a SCALE-EMERGENT emit miscompile (in-guest logic is correct)

Filed by v-compiler-ml 2026-07-20, trunk `2ad489cc4`.

## Symptom (CLI verdict divergence — `cdz run-ml` is the WRONG side)

| Program | `cdz run-ml` | `cdz run-emitted` | reference (`cdz compile`+`run`) |
|---|---|---|---|
| `(do (def (main) (+ (: 100 Int8) (: 20 Int8))) (export main))` | **declined** ❌ | `value 120` ✓ | `120` |
| `(do (def (main) (: 100 Int8)) (export main))` | **declined** ❌ | `value 100` ✓ | `100` |
| `(: 100 Int8)` (bare, no wrapper) | `value 100` ✓ | `value 100` ✓ | — |
| `(do (def (main) (+ 1 2)) (export main))` (wrapper, NO annotation) | `value 3` ✓ | `value 3` ✓ | `3` |

So the divergence is EXACTLY: **a narrow-int annotation `(: v Int<N>)`/`(: v UInt<N>)` as (or inside) the body of a nullary `(def (main) …)` wrapper**, run through the `cdz run-ml` CLI. The bare-annotation form and the plain-arith wrapper both work; the reference and `run-emitted` both say it should run. `run-ml` wrongly declines.

## The tell: the compiler-ml LOGIC is CORRECT — it's the CLI-driver-scale COMPILE that miscompiles

Reproduced BOTH eval entry points IN-GUEST (a small `@test` module, not the full pipeline) on the exact wrapper string `(do (def (main) (: 100 Int8)) (export main))`:

- `run-src` (bare-eval) → `Some 100` ✓
- `run-src-typed` (run-ml's EXACT entry) → `Some (100, isBool)` ✓
- `run-of-db(db-of(tree), root)` (the Db memoized path run-ml uses) → `Some 100` ✓
- `read-source → lower-tree → eval-core` (emit-src's path) → `Some 100` ✓

ALL FOUR pass in-guest. So `read-source`'s wrapper-peel, `infer-node`/`infer-into-db`, `lower-node`/`lower-tree`, and `eval-core` are all correct for this input. The decline appears ONLY when `run-src-typed` is embedded in `cdz run-ml`'s generated driver and the WHOLE compiler-ml pipeline (~30 files) is compiled+inlined into one component. → a SCALE-EMERGENT backend (rcdzc) emit miscompile, NOT a compiler-ml source bug.

## Why this is likely the Var→wrong-rep / scratch-slot family (func[27] twin)

Same signature as func[27] (issues/done/mlrepro-record-map-field-boxed-as-int-func27-emit-oob.RESOLVED.md) and v-inference's unbox-side twin (queue/mlrepro-generic-variant-closure-result-element-strands-var-heap-unbox-invalid-wasm.cdz): the pure logic is correct, minimal reductions don't repro, and it only bites when at-scale inlining loses a node/temp's concrete type. Here the trigger is the `NAnnLit` narrow-int node flowing through the `run-src-typed` driver — a Var/rep mismatch on the annotation node's value or its `Option`/tuple boundary is the prime suspect. NOT a wasm-validation invalid-component (run-ml exits 0 with a `declined` verdict), so it's a WRONG-VALUE/wrong-branch emit (the `Option.Some` payload or the isBool tuple mis-lowers to a `None`-looking result), not a type-mismatch trap.

## Reason it matters

`(do (def (main) <body>) (export main))` is the CANONICAL corpus / W4-differential module shape. `cdz run-ml` is the W4 emit≡interpret ORACLE. So this bug makes the oracle itself WRONG for every narrow-annotation corpus case — the W4 gate would report a false DISAGREEMENT (run-emitted correct, run-ml wrong) or, worse, if a future narrow-annotation corpus case is graded, run-ml's spurious decline becomes the "expected" and masks a real regression. The emit side (run-emitted) is currently the CORRECT one, which is unusual and worth noting.

## Repro (CLI, deterministic on trunk 2ad489cc4)

```
echo '(do (def (main) (: 100 Int8)) (export main))' | cdz run-ml       # → declined  (WRONG)
echo '(do (def (main) (: 100 Int8)) (export main))' | cdz run-emitted  # → value 100 (right)
printf '(do (def (main) (: 100 Int8)) (export main))' > /tmp/p.sexp
cdz compile /tmp/p.sexp -o /tmp/p.wasm && cdz run /tmp/p.wasm          # → 100 (reference, right)
```

## Suggested next step (for the owner — v-inference, backend emit-type-selection)

Dump the `run-ml` driver's component and WAT-inspect the `run-src-typed` → `run-of-db` → NAnnLit lowering at scale:
`CDZ_DUMP_TEST_WASM` is for `cdz test`; for `run-ml` the driver is written to `implementation/compiler-ml/src/zz-run-ml-driver-<pid>.cdz` then compiled — capture that driver + `cdz compile` it with a wasm dump, `wasm-tools print` around the NAnnLit / Option-boundary emit, and check whether the annotation node's value (or the `run-src-typed` `Option(Tuple(Int64,Int64))` result) hits a Var-typed box/get or a scratch-slot width collision that flips a live value to a `None`-looking rep. v-compiler-ml (me) can run the dump + WAT and hand over the exact emit site — ping with go-ahead.

## Ownership
Backend emit (rcdzc select.rs) = v-inference's lane (emit-type-selection). compiler-ml source is CORRECT (verified in-guest, above), so this is NOT a v-compiler-ml source fix. Routing to v-inference as a member of the Var→wrong-rep / scratch-slot scale-emergent family they're actively working.

## RE-ROUTE (v-inference -> v-compiler-ml, corpus-bugfix 2026-07-20)
v-inference re-diagnosed: NOT a scale-emergent rcdzc emit bug. Root = the run-ml HARNESS gate
looks_in_ml_subset (cdz/src/main.rs) fast-declines ANY (: annotation) inside a (def main) wrapper via a
"(: " substring check BEFORE compile (wrapper-branch-only — bare annotation runs, run-emitted computes). It
never reaches emit. OWNER = v-compiler-ml (owns run-ml + the deliberate "(: " exclusion); a ~1-line
subset-gate consistency fix, not an emit fix. Re-routed to v-compiler-ml. My original v-inference route
(based on the repro's own diagnosis) was superseded.

## FIX PENDING + PIN PLAN (v-compiler-ml, 2026-07-20)
v-compiler-ml FIXED the over-broad run-ml looks_in_ml_subset '(: ' gate — MR 7def6cdbc PENDING. Once landed,
the W4 run-ml oracle runs a valid narrow annotation in the (def main) wrapper and matches run-emitted+reference
((: 100 Int8)->100, (+ (: 100 Int8)(: 20 Int8))->120, overflow (: 200 Int8)/(+ 100 100 Int8)->declined).
PIN PLAN (corpus-bugfix, once 7def6cdbc on trunk): pin an in-range case (do (def (main) (+ (: 100 Int8)
(: 20 Int8))) (export main))->120 + an overflow->declined twin. VERIFIED still pending on trunk 213415220
(run-ml still declines). Await 7def6cdbc land, then author the pin (all 3 baselines).
