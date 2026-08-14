# 2026-08-13 4-tuple arm dispatch-count decline cliff (tick 1447)

The plt1 longest-plateau probe (4-tuple scalar state, arm = 3 chained lets with
2 ifs sharing the (> r2 bl) comparison + modulo-packed resume value) DECLINES
uniformly ×3 — but only at ≥4 DISPATCHES:
- `t4c.sexp` 2 dispatches → PASS
- `t4-3.sexp` 3 dispatches → runs (FAIL only on my placeholder expectation 0 vs 72)
- `t4-4.sexp` 4 dispatches → uniform todo ×3
- `plt1.sexp` the original 5-dispatch probe → uniform todo ×3
Also the nested-tuple variant (tuple prev run (tuple bl bv)) declines identically.
A specialized-fold depth cliff (same family feel as the OLD #23 dispatch-count
trigger, but a clean DECLINE not invalid wasm — the fold gives up rather than
mis-emitting). Witness bank; NOT filed as a bug (uniform clean decline = todo
surface). Candidates for flip-watch when fold depth extends.

## Tick 1448 — cliff REFINED: it's CHAINED-LET count × dispatch count, not tuple arity
| arm shape | dispatches | verdict |
|---|---|---|
| simple rotate (0 lets) 4-tuple | 5 | PASS (cl1) |
| 1 let + 1 if, 3-tuple | 5 | PASS (cl4) |
| 2 lets (2nd feeds state AND resume), 3-tuple | 3 | runs (cl5) |
| 2 lets, 3-tuple | 4 | todo (cl6) |
| 2 lets, 3-tuple | 5 | todo (cl3) |
| 3 lets + 2 ifs, 4-tuple | 4+ | todo (t4-4/plt1) |
CLIFF = (chained lets in arm ≥ 2 where a let feeds BOTH the resume value and
next-state) × (dispatches ≥ 4). The fold's per-site specialization cost grows
with arm let-depth (cf. finding #24's per-site continuation cloning — this
decline is likely the fold REFUSING where #24's growth would explode).

## Tick 1449 — the LOOP-DRIVEN twin PASSES ×3
- `plt2.sexp` — the EXACT same 3-let 2-if arm that declines at 4+ straight-line
  dispatches folds fine when ONE recursive driver (drive over a feed list)
  produces the dispatches: 5 feeds, argmax semantics intact (1424344454 /
  1424242437). Confirms the cliff is per-STATIC-CALL-SITE (matching #24's
  loop-immunity) and pins the WORKAROUND form. plt2 = pin candidate; plt1 =
  its flip-watch decline witness.
