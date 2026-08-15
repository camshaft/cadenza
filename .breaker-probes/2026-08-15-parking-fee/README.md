# prk1 — parking-lot fee meter (2026-08-15, tick 1534)

(entry-stamp, running-total) state: `enter` stamps the time; `exit` charges
the seed-shaped first-hour rate ((n%4)+2) plus 2 per further hour CAPPED at
15, with a zero-duration stay free (the -1 sentinel resets after each exit);
`rev` totals the day. Three stays exercise all three fee branches: a normal
stay (8 vs 6 — rate-differentiated), a zero-duration freebie (0 both), and
a long stay that hits the cap (15 both — the cap ERASES the seed difference
on that row while the totals still differ, 23 vs 21).

3-branch exit arm with the fee compound recomputed twice in one branch —
2-tuple, 3 dispatches through the branching arm: envelope-safe. PASS ×3.
**Pool (with tie1; +1 fills the 7th trio).**
