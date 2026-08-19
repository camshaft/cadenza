# nested-do-let-body — nested do-block as a LET body (printer-fix regression pin, let position)
## pynd2 — (let ((k 5)) (do A (do B (+ C k)))). Model 3005/2005. PASS x3, real-harness round-trip clean.
Exercises the fn/let greedy-body position of the 6345bd197 printer fix (complements pynd1's handle-body). Let binding survives into the innermost expr; discarded dispatches advance state. Promotable.
