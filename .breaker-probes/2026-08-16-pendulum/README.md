# pdl1 — pendulum with friction (2026-08-16, tick 1589)

(active, phase) 2-tuple: each swing hands the active quantity across losing
one (phase flips), a dead pendulum answers -1 forever. Tall drop (14) still
swinging at swing six (13…8); short drop (4) dies at the fifth with the
zero-crossing row (…1, 0, -1, -1) pinning the exact stop.

Frontier note: the first 3-tuple draft (h, s, phase with a sum-guard
(+ h s) reading two fields) DECLINED ×3 — the cross-field-guard family
again; merging h/s into one active field (the phase tracks which side)
compiles. Also a model-slip note: the 2-tuple python rewrite dropped a row
offset; the compiler's uniform 3-backend answer was the tell (fix the model,
not the compiler).

PASS ×3. **Pool — fills prs1/flt1/pdl1 (12th trio ready).**
