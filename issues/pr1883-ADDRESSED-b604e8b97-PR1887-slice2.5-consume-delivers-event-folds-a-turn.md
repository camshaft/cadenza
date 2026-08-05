# PR #1883 review comment — cdz-agent-host/tests/name_store_publish_consume_e2e.rs (v-agent-harness-host) — OPEN

https://github.com/camshaft/cadenza/pull/1883 (§4c publish→consume demo — resolve + run).

## Test claims it "RUNs one fold" but only checks the component LOADS, never executes fold (Copilot, name_store_publish_consume_e2e.rs:169) — test-precision [VERIFIED]
> The test/docs claim the resolved artifact "RUN[s] one fold", but the code only checks the component
> loads (`AsyncComponentReducer::from_component_bytes`) and never actually executes fold.
VERIFIED on the cand branch: the test ends at `AsyncComponentReducer::from_component_bytes(&fetched)
.expect("...loads as a runnable reducer component (fold.apply bound)")` — it verifies the published bytes
round-trip through the store + LOAD as a reducer, but never CALLS fold/fold.apply. So the "publish →
consume → run a fold" e2e stops at load; a regression that made fold itself fail on a store-resolved
component would pass green. Same "test doesn't exercise its claim" class as #1652/#1688/#1869. Either
execute a fold (deliver an event / call fold.apply) to match the "runs one fold" claim, or narrow the
claim to "loads as a runnable reducer". MED test-precision (this is the flagship publish→consume→run demo
— it should actually run). Fix-forward.
