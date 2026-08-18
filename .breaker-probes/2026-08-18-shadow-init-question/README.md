# RESOLVED: shadow-init draw + tolls — 7010 is CORRECT (2026-08-18)

v-effects RULING (note 039193): the observed values are correct semantics,
not a bug. Their independent oracle: the DISTINCT-EFFECT differential
(inner handler over F instead of E, no shadowing possible) computes the
IDENTICAL value — so the shadow value is pinned to the unambiguous
nearest-dynamic-handler semantics. My original model under-counted: the
outer arm's continuation is the ENTIRE rest of the outer body (the whole
inner handle), so when the inner region also draws, the inner arm's toll
threads through that same outer continuation and the outer post-resume
toll composes with it. red2 (pure inner body) has no such composition,
which is why it matched the naive model.

All three now carry RULED oracles and pass x2 backends:
- pysh3: 41010 / 27000 (full form: init-draw + shadowed draw + final draw)
- red1:  7010 / 4000   (minimal composition: init-draw + one shadowed draw)
- red2:  2007 / 1007   (control: pure inner region, naive model exact)

Lesson (ledger tick 1785): when a hand model diverges UNIFORMLY, the
distinct-effect differential is the oracle-of-oracles — it removes the
shadowing variable entirely. Model lesson: deep-handler continuations
scope to the WHOLE rest of the handled body, not the current expression.
