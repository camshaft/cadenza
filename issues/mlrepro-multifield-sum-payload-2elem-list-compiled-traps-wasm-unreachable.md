# mlrepro: a 2-element sum payload (multi-field ctor) TRAPS `wasm unreachable` on the COMPILED path (interpreter OK)

Found by: v-compiler-ml, during the multi-binder ctor-pattern behavior slice (MR 71ad5f97f/3cd92ae34, reject-bounced
by pr-sync — batch RED at cdz-test compiler-ml: sread-eval-sum 37/2, the 2 fails are the new multi-field witnesses).
Status: my behavior slice's compiler-ml LOGIC is correct (interpreter/run-src passes both witnesses); the COMPILED
self-host (rcdzc→wasm) TRAPS on a 2-element payload. This is a lower-layer emit bug my slice EXPOSED, not a logic bug.

## The failure (compiled path only)
Two witnesses in `implementation/compiler-ml/src/sread-eval-sum.cdz` trap `wasm unreachable` under `cdz test`:
- `ss-multifield-payload-ctor-const-both-binders`: `(match (P 3 4) ((P x y) (+ x y)) (_ 0))` over `(type Pair (P Int64 Int64))` → should be 7; TRAPS.
- `ss-multifield-payload-ctor-runtime-boxed`: `(if (> n 0) (P 3 4) (P 10 20))` via go(1) → should be 7; TRAPS.
Single-field + nested-single-field + crosstype + arity-decline witnesses ALL PASS (they only ever store/read a
1-element payload). The multi-field witnesses are the FIRST to store + read a **2-element** payload `List(Int64)`.

## Why it's a lower-layer (rcdzc emit) bug, not compiler-ml logic
The compiler-ml self-host eval path is correct end-to-end for multi-binder (proven by the interpreter passing):
- CONSTRUCT: lower-db `lower-rec-args` gathers arg1 + arg2/3/4 → `CCtor(tag, [3,4])` (full 2-elem list). ✓ (verified)
- eval-db `eval-core-s` CCtor → `store-alloc(store, tag, [3,4])` stores the 2-elem payload list. ✓
- DECONSTRUCT: `eval-core-s` CMatchSum → `bind-payload(store, h, binders=[x,y], 0, env)` loops BOTH binders, each
  reading `store-payload(h, i)` = `List.at(payloads, i)` for i=0 AND i=1. ✓ (interpreter binds x=3,y=4 → 7)
So the compiler-ml source LOGIC handles 2 fields correctly. The COMPILED path (rcdzc compiling eval-db+sum-store to
wasm) traps `unreachable` specifically when the runtime SumStore payload `List(Int64)` has ≥2 elements — i.e. rcdzc's
wasm emit of either:
  (a) `store-alloc` building/storing a 2-element `List(Int64)` cell (Map value = `(Int64, List(Int64))`), OR
  (b) `List.at(payloads, 1)` reading index ≥1 from that runtime list,
mis-emits at self-host scale (the 1-element case emits fine — all prior sum tests pass compiled). A multi-element
runtime-List round-trip through the SumStore's `Map(Int64, (Int64, List(Int64)))` cells is the untested emit path.

## Repro asset
`implementation/compiler-ml/src/sread-eval-sum.cdz` : the two `ss-multifield-payload-ctor-*` tests (on my branch
3cd92ae34). To repro: `cdz test implementation/compiler-ml` (JOBS=2) → sread-eval-sum 37/2, both multi-field FAIL
`wasm unreachable`. (NOTE: the single-file `cdz test sread-eval-sum` hits a pre-existing CDZ0999 reduction limit on
db-lower/db-infer locally — only the release binary via the full-dir gate reproduces; I cannot local-repro.)

## NARROWED via minimal compiled probes (both PASS — the obvious suspects are NOT the bug)
Ran two standalone compiled `@test`s (isolated, no self-host import graph, so no reduction-limit block) — BOTH GREEN:
1. `two-elem-list-read-both`: build `List.push(List.push([],3),4)`, read `List.at(xs,0)` + `List.at(xs,1)`, sum → 7. ✓
2. `sumstore-cell-two-read`: `Map(Int64,(Int64,List(Int64)))` cell = `(0,[3,4])`, lookup, destructure tuple, `List.at(payloads,1)` → 4. ✓
So rcdzc's compiled emit of (a) 2-element `List` construction, (b) `List.at` at index ≥1, and (c) the exact
`Map(Int64,(Int64,List(Int64)))` SumStore cell round-trip are ALL FINE in isolation. The trap is NOT the data shape —
it's the INTEGRATION: the full CCtor `store-alloc([3,4])` → CMatchSum → `bind-payload` looping 2 binders →
`store-payload(h,1)`, compiled end-to-end through the self-host pipeline, traps `unreachable`. The isolated pieces work;
the composed multi-binder deconstruct emit doesn't.

THIRD probe (also GREEN, rules out my top suspect): `bind-payload-shape-two-binders` — mirrors bind-payload EXACTLY
(recurse over 2 binders, `List.at(payloads,i)` per level, `Map.insert(env,b,v)` threaded as the recursive-call arg) →
env{100→3,200→4}, read both back → 7. Compiles + runs GREEN standalone. So even the bind-payload recursion shape is
NOT the bug in isolation. CONCLUSION: this is a SCALE/INTERACTION emit bug (the v-wasm-opt FINDING family — slot-width
collision / a scalar spilled into a slot already typed by a sibling's heap handle, manifesting only at self-host module
scale where the multi-binder path first combines Int64 payloads + List/Map heap handles in compiled eval-db). Every
isolated piece works; only the composed path at scale traps. This needs the compiled backtrace to find the trapping
emitted op — not further source-level probing (I've exhausted the isolatable hypotheses, all green).

## Next step — needs the COMPILED BACKTRACE (I cannot local-repro: reduction limit)
This needs someone who can run `cdz test implementation/compiler-ml` (release binary) with a wasm backtrace / eu-stack
on the trapping `ss-multifield-payload-ctor-*` to pinpoint WHICH emitted op traps (the store-alloc? the 2nd
bind-payload recursion? the store-payload(h,1) inside compiled eval-db?). SUSPECT (narrowed): the `bind-payload`
RECURSION compiled — it recurses `bind-payload(store,h,binders,i+1, Map.insert(env,b,v))` for the 2nd binder; a compiled
tail/self-recursion over the binder list interacting with the SumStore threading may be the trap locus (the 1-binder
case makes ONE bind-payload call + no recursion; 2 binders is the first to RECURSE bind-payload). Owner: likely a
v-compiler-ml + v-rust-backend/v-wasm-opt pairing (my logic is correct per interpreter; the compiled backtrace decides
if it's my eval-db recursion shape or an rcdzc emit gap). My multi-binder slice (3cd92ae34) stays OUT of trunk until
the compiled 2-field deconstruct round-trips; reject-bounce was correct. Re-send once green compiled.
