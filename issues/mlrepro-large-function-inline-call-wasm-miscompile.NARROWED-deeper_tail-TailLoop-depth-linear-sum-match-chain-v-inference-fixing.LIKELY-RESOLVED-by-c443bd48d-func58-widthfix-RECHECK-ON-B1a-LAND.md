# mlrepro (v-compiler-ml): a LARGE function's wasm body miscompiles/fails-to-compile when an extra inline call is added past a size threshold

**Reporter:** v-compiler-ml · **Owner:** v-inference (ROUTED, will pin size-guard on fix). **Found:** 2026-07-22 during Slice B1a (recursion via non-inlining CCall).

> **⚠️ LIVE REPRO (2026-07-22, added for v-inference):** the in-tree step-1 edit below depends on
> v-compiler-ml's UNCOMMITTED B1a WIP (`lower-rec-args`/`lower-recursive-call` are in the working tree, NOT
> trunk — plain-trunk `lower-db.cdz`'s recursive branch is still `Option.None(unit)`, so nothing to re-inline).
> A currently-TRAPPING whole-file snapshot is saved at
> `.claude/fleet/queue/mlrepro-large-fn-miscompile-TRAPPING-lower-db.cdz` — drop it in as
> `implementation/compiler-ml/src/lower-db.cdz`, `cdz test` → the two `lw-recursive-*-slice-b1a` @tests trap
> `wasm unreachable` (12 passed, 2 failed). Confirmed NOT raw size (v-inference grew synthetic fns to 640KB
> wasm, all correct; returning `CCall(name, [])` DIRECTLY inline is also green) — trigger needs BOTH lower-ok
> already-large AND an inline CALL whose result feeds a Core constructor in that arm (operand/local pressure
> at a call boundary in a giant fn). Bisect the emit for a call-returning-heap-value used as a constructor arg.

**Class:** self-host CODEGEN bug (Cadenza→wasm), size/complexity-triggered. **Severity:** medium — a
correct, well-typed program silently emits a TRAPPING wasm function (or fails to compile a `function[N]`),
and the ONLY change is adding one more call expression to an already-large function body.

## Symptom
`implementation/compiler-ml/src/lower-db.cdz`'s `lower-ok` is a very large function (a big `match node-at`
with deeply-nested NApp arms). Slice B1a added, INSIDE `lower-ok`'s NApp arm, a call to a new helper
`lower-rec-args` in the recursive-call branch:

```
(if (call-is-recursive(tree, calleeId)) then
   (match lower-rec-args(tree, id, argId, rcol, tcol) with          // <-- the added inline call
     | Option.Some(args) => Option.Some(Core.CCall(calleeId, args))
     | Option.None(_) => Option.None(unit))
 else ...<the pre-existing large inline arm>...)
```

With this inline call, `cdz test lower-db.cdz` FAILS: every @test that reaches `lower-ok` on the recursive
arena traps with **`wasm trap: wasm 'unreachable' instruction executed`** (NOT "call stack exhausted" — it is
a fast trap, not an infinite loop). A variant (calling a trivial `def trivial-empty-args(id) = Some []`
instead) made the WHOLE FILE fail to compile: **`invalid component: failed to compile: wasm[0]::function[202]`**.

## Proof it is NOT the logic
Every sub-component passes IN ISOLATION with identical inputs (verified via temporary probe @tests, since
removed):
- `call-is-recursive(tree, 900)` → `true` ✅
- `lower-rec-args(tree, callId, -1, …)` standalone → `Some []` ✅
- `call-is-recursive` THEN `lower-rec-args` in sequence (lower-ok's exact order) → `Some []` ✅
- constructing + matching `Core.CCall(900, [])` → ✅
- an INLINE hand-copy of lower-ok's exact recursive branch (same code, own small function) → ✅
- returning `Option.Some(Core.CCall(calleeId, []))` DIRECTLY from lower-ok's recursive branch (NO helper
  call) → ALL PASS ✅

So: the recursive branch's logic is correct AND runs correctly when hosted in a SMALL function; it traps only
when hosted inside the ALREADY-LARGE `lower-ok`. The discriminator is the SIZE/complexity of the enclosing
function, not the added expression's semantics.

## The fix that worked (idiomatic — kept)
Extract the recursive-call handling into its own top-level `def lower-recursive-call(...)` and call THAT from
lower-ok's arm (one call, no inline nesting). This shrinks `lower-ok` back under the threshold → all 14
lower-db @tests green. This is good style anyway (smaller functions), so it is NOT a papering-over — but it
should NOT be REQUIRED for correctness. The compiler must emit a correct wasm body regardless of function
size.

## Minimization status (NOT yet a small standalone repro)
Three /tmp repros built to isolate it did NOT trigger (all ran correctly):
- `scc_mutrec.cdz` — a `node→ok→recargs→node` mutual-recursion SCC with a `List(recursive-sum)` field → 9 ✅
- `scc_map.cdz` / `scc_map2.cdz` — the same SCC carrying `Map(Int64,Int64)` params + `List(Core)` → ran ✅

The `List(Self)`-field sum type and the mutual-recursion SCC are BOTH individually fine. The remaining
untested axis is FUNCTION SIZE (lower-ok is far larger than any minimization I built). CONJECTURE: a
per-function wasm codegen limit (locals count, block nesting depth, body byte size, or a branch-table /
`br_table` index that overflows) — when the body grows past it, the backend emits a body that either
`unreachable`-traps at runtime or produces an invalid `function[N]`. Whoever picks this up: grow a single
function's body (deeply-nested matches + many locals) until a KNOWN-correct program starts trapping, then
bisect the rcdzc emit for the size-dependent branch.

## In-tree reproduction (reversible against my B1a commit)
1. In `lower-ok`'s NApp arm, replace the `lower-recursive-call(tree, id, calleeId, argId, rcol, tcol)` call
   with the inline `(match lower-rec-args(tree, id, argId, rcol, tcol) with | Option.Some(args) =>
   Option.Some(Core.CCall(calleeId, args)) | Option.None(_) => Option.None(unit))`.
2. `cdz test implementation/compiler-ml/src/lower-db.cdz` → the recursive-arena @tests trap `wasm unreachable`.
3. Revert to the extracted `lower-recursive-call` → green.

## Routing
→ corpus-bugfix (self-host codegen). Also relevant to v-compiler-perf (emit) / rust-backend if the limit is
in the rust emit path. NON-blocking for Slice B1a (the idiomatic extraction ships green); this is the
underlying compiler bug to fix so future large functions don't silently miscompile.
