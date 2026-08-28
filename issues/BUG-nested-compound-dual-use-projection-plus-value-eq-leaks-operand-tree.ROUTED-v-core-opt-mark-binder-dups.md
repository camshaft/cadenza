# BUG (wasm reclaim): a NESTED compound that is both PROJECTED-into and passed to value-eq leaks its heap nodes

**Status:** OPEN — OWNERSHIP SETTLED (2026-08-27): v-core-opt claimed it (Core-IR reclaim placement — missing end-of-scope drop for a dual-borrowed binding via mark_binder_dups; folded into the §4 unified consuming analysis; operator concurred; v-memory-safety stays stopped). Originally — routed to `v-memory-safety` (rc/emit — same lane as
issues/hardening-valueeq-emit-does-not-box-a-scalar-operand-at-erased-heap-type.md, which is a
different face of the value-eq emit). FYI v-core-opt (Perceus placement design).

**Found by:** breaker (tick 295, 2026-08-27), while probing nesting-depth static fences. Values are
CORRECT in every cell; the defect is census-only (leak). Runtime `051neLQ`/debug `05OU7cA` (post
mark-immortal-deep #4301), fresh `cdz` (post-#4304 trunk).

## The isolation matrix (all values correct; census = last line)

| cell | shape | census |
|---|---|---|
| projections only, depth-3 nested runtime tuple | `(. (. (. a 1) 1) 1)` reads, no eq | **0** |
| eq only (×2 calls), depth-3 nested runtime tuples | `(= a b)` + `(= b a)`, no projections | **0** |
| unequal-leaf eq, depth-3 | early/full walk, no projections | **0** |
| FLAT dual-use | `(. a 0)`, `(. b 1)` AND `(= a b)` | **0** |
| ONE side dual-used (deep-projected + eq'd), other eq-only | depth-3 runtime | **3** = the dual-used side's full node count |
| BOTH sides dual-used | depth-3 runtime | **6** = 3 + 3 |
| both dual-used, runtime OUTER + constant inner subtree | depth-3 | **2** (immortal inner nodes census-excluded) |
| both dual-used, tuple with a small-LIST child | depth-1 compound child | **3** = tuple + vec + arr of the deep-projected side |

Model: when a nested compound operand of `=` is ALSO consumed by a projection chain, the dup taken
for the second use is never released — the whole operand tree (every node reachable from the
dual-used binding) stays live. Flat operands are immune; each dual-used SIDE leaks independently;
constant (immortal) nodes are excluded from the reading but mortal re-materialized nodes still leak.

## Canonical repro (census 6, value 101001)

```
(do (def (main (: n Int64))
  (let ((a (tuple n (tuple n (tuple n n))))
        (b (tuple n (tuple n (tuple n n)))))
    (+ (* 1000 (. (. (. a 1) 1) 1)) (+ (. (. b 1) 0) (if (= a b) 100000 0)))))
(export main))
```

Run: `cdz run repro.wasm --arg 1 --runtime <DEBUG>.wasm --report-live-objects`.

## Pinned acceptance (corpus, batch 522, 03-equality)

`dqe1`-`dqe5` in `spec/semantics/03-equality-and-observation.sexp`: three 0-census CONTROLS
(flat dual-use / eq-only / unequal) that must STAY 0 through any fix (an over-fix that
over-drops corrupts the projection reads — the values are the fence), and two known-leak
calibration rows (one-side 3, both-sides 6) that flip to 0 on the fix. Zero post-landing pinning.

## GENERALIZATION (breaker, tick 296, 2026-08-27) — NOT value-eq-specific: projection + ANY heap-walking consumer

Second-consumer sweep on the same depth-3 runtime-tuple shapes (values correct in every cell):

| second consumer alongside the projection chain | census |
|---|---|
| `(= a b)` (value-eq) | 3 (the original finding) |
| `(< a b)` (blessed lexicographic order walk) | **3** |
| `(Map.lookup (Map.insert (Map.empty) a 42) b)` (champ key hash+eq descent) | **3** |
| `(Set.contains (Set.of (list a)) b)` (set membership descent) | **3** |
| CONTROL: `(< a b)` alone, no projection | 0 |
| CONTROL: `(= a b)` AND `(< a b)` — TWO walkers, no projection | **0** |

Refined model: the walkers are innocent (any number of them borrow cleanly). The leak needs a
PROJECTION chain plus at least one walking consumer of the same nested binding — the dup
materialized for that combination is never released, leaking the operand's full tree (per
dual-used side). The fix is in the generic dup/drop placement for projected-and-walked nested
compounds, NOT in the value-eq emit specifically — fixing eq alone would leave order/champ/set
leaking (dqe6-8 pin those cells; dqe9 pins the two-walker clean control).

Pinned acceptance extended (batch 523): `dqe6`/`dqe7`/`dqe8` known-leak 3 → 0 on fix;
`dqe9` two-walker control stays 0.

## ESCAPE cells (breaker, tick 297, 2026-08-27) — for the mark_binder_dups end-of-scope-drop fix

Ownership settled: v-core-opt (Core-IR reclaim placement — missing end-of-scope drop for a
dual-borrowed binding via mark_binder_dups), operator concurred. Three cells probing exactly that
fix's edge, measured pre-fix:

| cell | shape | census today |
|---|---|---|
| walker + operand ESCAPES whole | `(if (= a b) a <const>)` returned; caller projects it | **6** — BOTH sides' trees |
| walker + a COMPONENT escapes | `(if (= a b) (. a 1) <const>)` returned | **6** — both sides again |
| eq-only in a callee scope, nothing escapes | walker alone, scalar out | 0 |

New information vs the earlier matrix: an eq'd operand that ESCAPES through a branch arm leaks
BOTH operands' trees (in the projection cells only the dual-used side leaked). So the escape cell
is an EXISTING under-drop, not merely a future over-drop hazard. The end-of-scope drop must
discriminate: DO drop the dead sibling (b), do NOT drop the escapee (a) — the pinned VALUES
(1001 read through the returned tree in the caller) fence the UAF side; the known-leak 6 clauses
flip to 0 on the correct fix. Pinned as dqe10/dqe11 (+ dqe12 the callee-scope 0-control),
batch 524.

## KIND/CONSUMER narrowing (breaker, tick 298, 2026-08-27) — leg 1 is TUPLE-positional-projection specific

Operand-kind and consumer-kind sweep on the dual-use shape (walker = eq throughout; heap children
forced where noted to kill the scalarization confound):

| extracting consumer + walker | census |
|---|---|
| tuple `.` index chain (depth 3, scalar leaf) | 3 (dqe4) |
| tuple `.` index, HEAP component at depth 1 (`List.len (. a 1)`) | 3 (tick-295 v7) |
| tuple MATCH-destructure, same depth-3 shape as dqe4 | **0** |
| record field read `(. a y)`, HEAP field (`List.len`) | **0** |
| Option match-extract, HEAP payload (`List.len`) | **0** |
| record/Option of scalars (possibly scalarized; kept as breadth) | 0 |

So the under-drop family has TWO distinct legs:
1. **Tuple positional projection of a component + walker on the same binding** — leaks the
   projected side's tree. Record `.` (named field), sum match-extract, and tuple match-destructure
   all release correctly — the dup minted by the TUPLE-INDEX read is the one never dropped.
2. **Walker-result branch whose arm ESCAPES an operand** — leaks BOTH sides (dqe10/dqe11), no
   projection involved.

Pinned dqe13-15 (batch 525) as 0-CONTROLS documenting the discrimination: they must STAY 0 and
their values must hold if the fix touches the match/record paths (an over-drop there corrupts the
reads these cases make after the walker).

## Leg-2 trigger table (breaker, tick 299, 2026-08-27)

| cell | census |
|---|---|
| walker-cond branch, taken arm escapes an OPERAND (dqe10/11) | 6 = escapee ×2 |
| walker-cond branch, taken arm escapes a NON-operand heap binding | **2** = the escapee's tree once |
| walker-cond branch, escape arm UNTAKEN (operands unequal) | 0 |
| SCALAR-cond branch, taken arm escapes a heap value | 0 |
| walker-cond branch, scalar arms (dqe2/dqe9) | 0 |

Leg-2 exact trigger: **the CONDITION is a walker result AND the taken arm yields a heap value —
that escapee's tree leaks once (twice when the escapee is also a walker operand).** The dup is
minted on the taken path only (untaken arm clean); a scalar condition never leaks. So both legs
are dups minted in the shadow of a WALKER call that end-of-scope never releases — leg 1 keyed by
tuple-index extraction, leg 2 by branch-arm escape under a walker condition.

Cross-scope note: leg 1 reproduces on a RETURNED binding in the CALLER (projection + walker on
`r = f(n)` leaks r's tree, 3) — the fix must key on the binding's consumers, not on where it was
constructed. Pinned dqe19.
