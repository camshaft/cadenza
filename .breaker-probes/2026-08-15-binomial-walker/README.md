# cmb — binomial walker: COMPILE HANG (2026-08-15, tick 1558) — FINDING

cmb1 (binomial-coefficient walker: C(N,k) incrementally via
c' = c*(N−k)/(k+1), the compound recomputed FIVE times across the two
branches — resume value + tuple fields on both sides of the mx comparison,
plus the mx guard itself) does not decline and does not explode —
**the compiler HANGS**: `cdz compile` runs past a 300-second timeout with no
output (exit 124), and the gate reports 'compile timeout (hang)' ×3.

cmbB — the same arm with the compound HOISTED through a match binder
(arithmetic scrutinee, fence-safe) — declines CLEANLY (todo), so the hang
needs the 5-way in-branch recompute; the binder form hits the ordinary
frontier instead.

Distinct from every prior face: not invalid wasm (emit never finishes), not
a decline, not an emit-size cap — a non-terminating compile. The (a)/(rust)
budgets can't catch what never reaches emit. Suspect: the partial-eval
sharing walk revisits the 5-recompute DAG exponentially WITHOUT the
memo/budget that the emit walk now has.
