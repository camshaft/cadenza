# grw1 — plant growth with pruning stress (2026-08-16, tick 1597)

(height, rate) state: `day` grows by the current rate; `prune h` cuts back
to the target answering the clippings AND slows the rate by one (bottoming
at one) — but ONLY when it actually cut (a no-op prune answers 0 with state
untouched). The fast grower (rate 4) is pruned twice, stressing its rate
down 4→3→2; the slow one (rate 2) SKIPS the first prune entirely (zero-clip
row, rate preserved) and only stresses on the second — conditional state
mutation where the CONDITION's outcome feeds back into future growth rows.

PASS ×3. **Pool — fills drm1/grw1 toward the 13th trio (+1).**
