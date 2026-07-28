# NMatchCtor binder-LIST schema diff (multi-binder pattern slice) — for v-inference sanity-check before the behavior slice

Per v-inference's request: the node-shape diff FIRST, so they can check the arity/List plumbing on the infer arm
before we build the behavior. Blocked on my `e4e49dc72` (multi-binder-declines) landing first — this edits the same
decline site.

## The one schema change

`parse-db.cdz:66` — `NMatchCtor`'s 3rd field goes from a single binder-id to a binder-id LIST:

```
- | NMatchCtor(Int64, Int64, Int64,        Int64, Int64)   // (scrutId, patCtorName, binderId,   bodyId, restId)
+ | NMatchCtor(Int64, Int64, List(Int64),  Int64, Int64)   // (scrutId, patCtorName, binderIds,  bodyId, restId)
```

That's it — one field. Everything else is arm updates at the destructure/construct sites below.

## Why this is LOW-risk: the lower target is ALREADY multi-slot

`CMatchSum` (lower-db.cdz:58) is ALREADY `CMatchSum(Core, Int64, List(Int64), Core, Core)` — a binder LIST. And
lower already calls `lower-binder-list(binderId)` (lower-db.cdz:201) which today wraps the single id:
`(if binderId == -1 then [] else List.push([], binderId))`. So the eval/emit deconstruct path already consumes a
binder list — the multi-binder value semantics are already wired downstream. The slice is really just: carry the
list from the reader through infer to the already-list-shaped CMatchSum.

## The ripple — every NMatchCtor site (construct = C, destructure = D)

| file:line | site | change |
|---|---|---|
| parse-db.cdz:66 | the type decl | field 3 `Int64`→`List(Int64)` |
| **sread.cdz:736** (D→C) | MY reader, single-binder build | loop binders into `List(Int64)` at the current decline site; build `NMatchCtor(…, binderIds, …)` |
| sread.cdz:698 (C) | MY nullary-payload `[]` build | `0 - 1` → `[]` (empty binder list) |
| parse-db.cdz:187,263 (D) | self-call scan (binder unused, `_bn`) | `_bn` now binds a `List` — no logic change, just the pattern arity |
| resolve-db.cdz:79 (D) | **v-inference** — resolve each binder in body scope | iterate the list, add each binder to scope (was one) |
| infer-db.cdz:117 (D) | **v-inference** — seed binder type | `binder[i]` ← `List.at(argTypes, i)` via ctor-argtypes-of (was single from ctor-argtype-of) |
| infer-db.cdz:1502 (C) | **v-inference** — rebuild after taint check | thread `binderIds` list through unchanged |
| lower-db.cdz:178 (D→C) | MY lower arm | pass `binderIds` list to `lower-binder-list` (generalize it to take a List, or map over it) |
| lower-db.cdz:740,756 (C) | MY nullary-enum builds | `0 - 1` → `[]` |
| lower-binder-list (lower-db.cdz:350) | MY helper | change signature `Int64`→`List(Int64)`: lower each binder id to its slot; today's single-wrap becomes identity-ish over the list |

Test sites (sread.cdz:1482/1511/1583) destructure with `_` for the binder field — arity-only, no logic change.

## Division (per v-inference's plan)

- **v-inference**: resolve-db:79 (bind each), infer-db:117 (seed `binder[i]`←`argTypes[i]`; a `-1` field → TErr that
  binder → declines, same discipline as single-field Bool), infer-db:1502 (thread list); arm-join / cross-type-reject
  / TErr-scrutinee compose over the list unchanged.
- **v-compiler-ml (me)**: sread reader (loop binders into List at the decline site) + lower (pass list to
  `lower-binder-list`, generalize that helper) + the nullary `[]` builds.

## Gate (v-inference's Bool-lesson steer — MANDATORY)

Compiled (`cdz test`) on a RUNTIME + BOXED witness — a multi-variant sum, runtime scrutinee so it can't fold:
`(match (if c (P 3 4) (Q 5)) ((P x y) (+ x y)) …)` must **RUN → 7** compiled. A const `(P 3 4)` folds + a
single-variant erases → both skip the store/extract path (the false-green trap). Not `run-src`, not a const.

## Sequencing

1. `e4e49dc72` (multi-binder-declines) lands. ← current blocker
2. This schema change as its OWN increment (schema + arm plumbing, behavior = still declines / single-binder works)
   — or folded into the behavior slice; v-inference's call on whether they want the schema landed separately first.
3. Behavior slice: reader loops binders + infer seeds per-field + lower carries the list → the runtime+boxed witness runs.
