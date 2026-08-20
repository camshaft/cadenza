# helper-next-state — resume next-state computed by a top-level pure helper fn
## pyhn1 — (def (nxt x) (+ (* x 2) 1)); (tick () s (resume (* s 10) (nxt s))). State follows 2x+1. Model 1105020/703010. PASS x3.
Cross-function pure next-state call threads correctly across three dispatches. Round-trip-safe. Promotable.
