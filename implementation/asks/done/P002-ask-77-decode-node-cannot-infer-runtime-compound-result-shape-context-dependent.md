## 77. ✅ CLOSED 2026-07-08 (seed loop) — `decode` return-kind mis-inferred SCALAR → the front-end declines on real bytes. Two faces of ONE ask-73-family bug (mutual-recursion heap-carrying tuple); CONTEXT-DEPENDENT (not minimally isolated)

**RESOLVED.** Root cause: the KIND-INFERENCE `match` arm never bound TUPLE-pattern binders, so
`decode`'s `((tuple ast pos) ast)` inferred `ast` (and thus `decode`'s result) as scalar Int64 instead of
Heap — both faces (scalar-slot "cannot infer runtime compound result shape" + heap-slot "runtime match
with a non-literal pattern") follow from that one mis-inference. Fix: bind irrefutable-tuple-pattern
binders in `InferCtx::infer`'s match arm via `scrutinee_tuple_slot_kinds` (heap slot → Heap, scalar cursor
→ Int), guarded to a CALL-returned tuple (not an inline `(tuple n 9)`); plus `tuple_slot_scalar_kind` now
falls back from `shape_of` to Kind inference so a recursive scalar slot producer (`skip-item`) is
recovered. Both faces compile against real cdzc.cdz; a standalone mutual-recursion regression case added to
`02-binding-and-control`. Memory: `ask77-mutual-recursion-tuple-return-kind-inference`. Details below.

---


**Status: the sole blocker between cdzc's front end and real-bytes `compile-bytes`.** With ask-73
(tail-recursive tuple return) and ask-76 (`List.concat`) fixed, the CBOR→Ast parser (`cdzc/15-decode.cdz`)
now COMPILES, but running the pipeline on REAL program bytes declines INTERNALLY at the very first stage, so
`(compile-bytes <real AST bytes>)` yields a TRAPPING component (cdzc self-compiles fine — the internal
decline lowers to a clean trap).

### The precise first decline: "runtime match with a non-literal pattern" (codegen.rs:7061)

`resolve-program` matches `decode`'s result: `(match (decode b) ((Ast.List xs) …) ((Ast.Int n) …) …)`. On
real bytes this declines **"runtime match with a non-literal pattern"** at `codegen.rs:7061`
(`gen_match_arms` — the SCALAR match path: it reached a ctor/`Ast.List` pattern over a scrutinee it
classified as scalar `Int64`, and a ctor pattern is "non-literal" there).

**Root cause — `decode`'s result kind is inferred SCALAR when it is actually a HEAP `Ast`.** `decode`
returns element 0 of `decode-node`'s `(tuple <Ast>, Int)` via `((tuple ast pos) ast)` (`15-decode.cdz:137`).
`decode-node` is mutually recursive with `decode-app-children`. The heap `Ast` slot's kind is lost across
that mutual recursion, so `decode` is treated as scalar and the caller's `Ast.List` pattern has no heap
scrutinee to destructure.

### It is the ask-73 / ask-14 family (coarse-kind return-inference re-derived at emit) — TWO FACES

The SAME mis-inference shows two different messages depending on which tuple slot the caller keeps:
- **extract the HEAP node (slot 0)** — what `decode` does → **"runtime match with a non-literal pattern"**;
- **extract the SCALAR cursor (slot 1)** — e.g. `((tuple ast pos) pos)` → **"cannot infer runtime compound
  result shape."**
(An earlier draft of this ask filed only the slot-1 "cannot infer…" face; the slot-0 "non-literal pattern"
face is the one that actually blocks the front end, since `decode` keeps the node.)

ask-73's fix (`scrutinee_tuple_slot_kinds`) recovers slot kinds through **direct tail-recursion**; the cdzc
chain is **mutual recursion** `decode`→`decode-node`↔`decode-app-children`, which that navigator doesn't
follow. So this is the mutual-recursion sibling of ask-73. Durable fix = real HM (ask-75).

### ⚠ NOT minimally isolated — resists standalone reduction (the ask-74 lesson)

Every reduction I built COMPILES standalone (verify a decline reproduces standalone before claiming a
minimal repro):
- a fn returning slot 0 (heap) OR slot 1 (scalar) of a `(tuple <Ast>, Int)`, caller matches it;
- the **exact** 3-fn mutual recursion `dn`(`decode-node`)↔`dac`(`decode-app-children`)/`dec`(`decode`)
  building built-in `Ast.Int`/`Ast.List` and matching the result over all 6 built-in-`Ast` variants.

It reproduces ONLY inside the full merged `cdzc.cdz` (interaction with the other Ast consumers —
`find-main-body`, `resolve-app`, `name-head-is`, etc. — or the module kind environment), exactly like the
retired ask-74. **No reliable standalone repro; I did not ship a false one.**

**Repro in situ (reliable).** Inject into `cdzc.cdz`, before the final `)`:
```
(def (main)
  (match (decode (Bytes.of (list <32 bytes = Ast.encode of "(module c (def (main) 42))">)))
    ((Ast.List xs) (List.len xs))
    ((Ast.Int n) n) ((Ast.Str s) 0) ((Ast.Name x) 0) ((Ast.Bool b) 0) ((Ast.Float f) 0)))
```
`emit` → "runtime match with a non-literal pattern". (The 32 AST bytes:
`83 01 84 61 63 63 64 65 66 64 6D 61 …` — obtain via `(Ast.encode (quote (module c (def (main) 42))))`.)

**Bisection recipe.** Start from the standalone `dn`/`dac`/`dec` cluster that COMPILES, then add the other
`cdzc.cdz` Ast/Hir consumers (`find-main-body(-go)`, `resolve-main-body`, `resolve-app`, `name-head-is`,
`lower`) one at a time until `(match (decode b) …)` flips to the decline — the addition that flips it is the
interaction to minimize.

### What's NOT blocked (verified this cycle)

The BACKEND works: from a hand-built Mir, `select`→`serialize`→frame emits the byte-correct scalar component
(`MInt 42`→2 body bytes, runs 42) and overflow-TRAPPING checked `+`/`-` satisfying the value/trap oracle
(1+2→3, 10-3→7, 5--3→8, nested→2; traps Int64.max+1 / Int64.min-1 / Int64.max--1). Only the `decode`
front-end blocks feeding REAL bytes through.

**Priority.** 🟡 blocks the front-end end-to-end (real-bytes `compile-bytes`). Seed inference gap in the
ask-73/ask-14 family: return-kind for a MUTUALLY-recursive heap-carrying tuple. Related: ask-73 (tail-rec
tuple, fixed — this is its mutual-recursion sibling), ask-74 (retired — same context-dependence discipline),
ask-75 (real-HM design — the durable fix), the coarse-kind inference learning
(`spec/learnings/2026-07-08-a-coarse-kind-classifier-re-derived-at-emit…`).
