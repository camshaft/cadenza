# LEAK (real, on trunk): a capturing closure's cell is not reclaimed after `call` dispatch

Filed by v-memory-safety 2026-07-18. Found while auditing why the `..._leaves_no_live_objects` leak-probe
family is NOT wired into the gate (all 72 are `#[ignore]`d, manual-only): running the family surfaced
**1 real failure on clean trunk** (840efdb83), value-correct so no value gate catches it.

## Symptom
`tests::closure_host_resource::a_closure_call_leaves_no_live_objects` (`#[ignore]`d, tests.rs ~71760):
make a capturing closure `(def (adder (: k Int64)) (fn ((: x Int64)) (+ x k)))`, host calls `make(10)`
(allocates a cell holding k=10) then `call(5)` (dispatches `(+ x k)` = 15). Value is CORRECT (15) but
**live-objects = 1** after the round-trip (expected 0). The closure cell is LEAKED — `call` borrows/reads
the captured env out of the cell but does not DROP the cell after dispatch. Per the probe's own doc: "call
owns the own<t> handle and must drop the cell after dispatch."

## Territory + scope
v-memory-safety (Perceus/rc discipline the compiler emits) — the closure-cell reclaim in the `call`
dispatch path. BUT it borders the host-closure/resource ABI (own<t> handles, the C-HOST-N resource-escape
series) — coordinate with v-effects / whoever owns the host-closure resource marshaling before/while fixing.
The cell is `arr-alloc(1 + captures)` (slot 0 = boxed code idx, then boxed captures — see the `Core::Closure`
emit); `call` reads the code slot (`arr-get` + `get-int` → table index) + `call_indirect`, reading captures,
but never drops the own<t> cell it received. FIX = drop the closure cell after the `call_indirect` dispatch
(the resource dtor / call path owns the handle), mirroring the owned-temporary reclaim pattern; verify no
UAF of a still-referenced capture (the captures are read INTO the call frame before the drop).

## Why it matters + the meta-finding
The whole `..._leaves_no_live_objects` leak-probe family (perceus_balance, SumExpect, MatchSum, value_eq,
string/bytes rope, bigint, resource-escape, closure, …) is `#[ignore]`d and run by NO gate phase / CI job —
so a leak regression is invisible to `cargo xtask check`. This closure leak has likely been latent since the
probe was added (093b35459). FOLLOW-UP (v-memory-safety, after this leak is fixed): wire the leak-probe
family into `cargo xtask check` (a step running `cargo test -p rcdzc --lib -- --ignored <the probes>` after
the debug-counters store is built) so the rc-invariant gate runs for everyone — can't wire it in while one
probe fails (it would red the gate), so this leak is the blocker for that gate-coverage improvement.

Repro: `cargo xtask build` then `cargo test -p rcdzc --lib a_closure_call_leaves_no_live_objects -- --ignored`.
Related: the SumExpect (ba9170945) + MatchSum (cbd1b35ab) owned-shell leak fixes (same reclaim discipline).
