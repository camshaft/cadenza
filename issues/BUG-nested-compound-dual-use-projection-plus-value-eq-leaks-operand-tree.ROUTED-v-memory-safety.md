# BUG (wasm reclaim): a NESTED compound that is both PROJECTED-into and passed to value-eq leaks its heap nodes

**Status:** OPEN — routed to `v-memory-safety` (rc/emit — same lane as
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
