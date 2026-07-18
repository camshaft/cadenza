# HARDENING: wire the `..._leaves_no_live_objects` leak-probe family into the gate + reconcile 2 contradictory closure probes

Filed by v-memory-safety 2026-07-18 (CORRECTS an earlier "real closure leak" filing — it is NOT a real
leak; see below).

## The meta gap
The entire `..._leaves_no_live_objects` leak-probe family (~12: perceus_balance, SumExpect, MatchSum,
value_eq, string/bytes rope, bigint, resource-escape, closure round-trips, …) is `#[ignore]`d (needs the
debug-counters store + `-- --ignored`) and run by NO `cargo xtask check` phase or CI job. So a genuine leak
REGRESSION is invisible to the gate — the direct rc-invariant probes never run automatically. Wiring them in
(a check step running the `--ignored` leak probes after `xtask build` populates the debug store) would make
the rc-leak gate run for everyone — the durable improvement.

## BLOCKER: two contradictory closure probes (must reconcile BEFORE wiring)
Running the family surfaced ONE failure — but it is NOT a real leak, it is a STALE/ASPIRATIONAL probe:
- `a_closure_call_leaves_no_live_objects` (tests.rs ~71760) asserts **live-objects == 0** after a single
  make+call-WITHOUT-drop, with the doc "call owns the own<t> handle and must drop the cell after dispatch."
  This describes the `own<t>` self-drop shape. FAILS (live=1).
- `a_runtime_closure_leaks_exactly_one_cell_known_gap` (tests.rs ~4937) asserts **live-objects == 1** and
  documents it as the ACCEPTED known gap: "a heap closure param threaded through recursion leaks exactly 1
  cell (flip to 0 when the general Perceus param-drop pass lands). A count > 1 is a REGRESSION." PASSES.

These CONTRADICT. The current-reality one is the `known_gap` (=1): PRODUCTION closure `call` is `borrow<t>`
(mod.rs 4331/4401, `call_borrow=true`, "the shared list-`call` takes borrow<t> (repeatable)"), so the host
KEEPS the handle after a single `call` and the cell is reclaimed by the `t-dtor` on DROP — NOT by `call`.
`a_round_trip_leaves_no_live_objects` (which DOES drop the handle) PASSES with 0. So live=1 after a
single-call-without-drop is CORRECT for `borrow<t>`; the `assert 0` probe is aspirational (the `own<t>`
end-state) / stale (written before `call` became borrow). ⚠ "Fixing" it by adding a cell-drop in `call`
would DOUBLE-FREE under borrow+dtor (a UAF) — do NOT.

## Action (v-memory-safety, when picked up)
1. RECONCILE the aspirational probe: either delete `a_closure_call_leaves_no_live_objects` (superseded by
   `a_runtime_closure_leaks_exactly_one_cell_known_gap` + `a_round_trip_leaves_no_live_objects`), or change
   it to DROP the handle before measuring (asserting 0 like the round-trip), or re-doc it as
   `#[ignore]`-and-assert-1 pending the Perceus param-drop pass. Coordinate with whoever owns the
   host-closure resource ABI (v-effects) since it documents an ABI intent.
2. THEN wire the (now all-green) leak-probe family into `cargo xtask check` as a step running
   `cargo test -p rcdzc --lib -- --ignored <the leaves_no_live_objects probes>` after the debug store builds
   — a fleet-wide rc-leak regression gate. Wire into `check` (not just CI) so pr-sync re-gates it too.
3. The real closure-cell leak-to-0 is the separate, ALREADY-TRACKED general Perceus param-drop pass (the
   `known_gap` test's "flip to 0 when it lands") — NOT this ticket.

Repro: `cargo xtask build` then `cargo test -p rcdzc --lib -- --ignored leaves_no_live_objects` → 1 fails
(the aspirational probe), the rest pass.
