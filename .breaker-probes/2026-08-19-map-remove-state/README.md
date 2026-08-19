# map-remove-state — Map-state handler with remove-then-lookup across the seam
## pymx1 — rm(k) threads Map.remove (answers shrunk len); later get(k) of removed key returns None, surviving key returns value. Model 5999/4999. PASS x3, ML-round-trip clean.
Delete-then-lookup consistency through the resume seam. Promotable.
