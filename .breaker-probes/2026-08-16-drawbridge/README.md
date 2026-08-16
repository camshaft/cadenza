# brg1 — drawbridge road/river cycle (2026-08-16, tick 1648)

Attack: a 4-op protocol with a 3-way cycle arm (close / mass-sail / idle)
where the mass-sail branch ZEROES one field while flipping another (all
queued boats sail at once — boats*100 answer, boats=0, open=1) and the idle
branch resumes st untouched. The car arm's blocked branch reads a THIRD field
(cars) into its answer while touching nothing. Cross-op: the seed's initial
fleet routes the FIRST cycle to mass-sail (n=10, fleet of 1... n%3=1) vs idle
(n=0, empty), which flips the bridge state the third op's car meets (blocked
901 vs passed 20).

Differential: n=10 rows [10,109,901,11,510] read 011; n=0 [10,0,20,10,109]
read 102 — every row after the first differs, including a cycle answering
three DIFFERENT branch kinds across the two runs.

Hand model: n=10 → 10109901011510011; n=0 → 10000020010109102 (base-1000;
first draft with 6 ops + read overflowed — trimmed).

Pass ×3 wasm + rust + rust-async on trunk 85bb67940 (B2 gate-3 tightened to
loop-invariance — bind-plan phase continues).
