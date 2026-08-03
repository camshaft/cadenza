# PR#1014 review comments (×3) — fuel-bound apply error mis-classification + roadmap-y field comment (v-agent-harness)

Mirrored from GitHub PR#1014 review comments (Copilot), ids `3696016968` (wasm_host.rs:229),
`3696016975` (:324), `3696016981` (:103). `cdz-kernel` → v-agent-harness. Blame `4782e74cc`
"feat(cdz-kernel): fuel-bound ComponentReducer::apply — runaway-guest DoS guard (§22d, PR#1009)" — the
fuel fix that closed my PR#1009 runaway-guest finding; these are follow-ons on it. Gate = cdz-kernel own
`cargo test`+clippy.

## Comment 1 (verbatim) — :229, Engine::new mis-classified

- (id 3696016968) "`Engine::new(&config)` failure is currently mapped to
  `ComponentError::InvalidComponent`, but that variant documents invalid *bytes*. Engine creation can
  fail for reasons unrelated to the component input (e.g. configuration/platform), so this error should
  be classified as an instantiation/host setup failure to avoid misleading callers."

### Liaison verification (confirmed on trunk 71c8856d7)

wasm_host.rs:227-228: `wasmtime::Engine::new(&config).map_err(|e|
ComponentError::InvalidComponent(e.to_string()))?`. `InvalidComponent` documents invalid component BYTES,
but `Engine::new` failing is a config/platform/host issue (nothing to do with the component). Map it to a
host-setup/instantiation variant (`Instantiate` or a new `HostSetup`) so a caller doesn't read a platform
failure as "your component is malformed". Error-classification.

## Comment 2 (verbatim) — :324, set_fuel mis-classified as Trap

- (id 3696016975) "If `Store::set_fuel(self.fuel_budget)` fails, it's a host-side setup error (e.g. fuel
  metering not enabled), not a guest trap. Mapping it to `ComponentError::Trap` makes the error variant
  misleading and conflates host failures with guest semantics."

### Liaison verification (confirmed on trunk 71c8856d7)

wasm_host.rs:322-324: `store.set_fuel(self.fuel_budget).map_err(|e| ComponentError::Trap(e.to_string()))?`.
`Trap` = a GUEST trap (the guest's totality contract broke). But `set_fuel` failing is a HOST-side setup
error (fuel metering not enabled on the engine — a config bug, not guest behavior). Mapping it to `Trap`
tells the driver "the guest trapped" when the host mis-configured. Map to `Instantiate`/host-setup. (Note:
`consume_fuel(true)` IS set on the engine at :226, so `set_fuel` shouldn't fail in practice — but the
classification is still wrong for the error path.) Error-classification.

## Comment 3 (verbatim) — :103, roadmap-y field comment

- (id 3696016981) "The struct field comment includes forward-looking/implementation-history notes (e.g.
  'interim… toward full gas (§22a) … arrives with async substrate') that are likely to go stale and
  don't describe current behavior. Prefer documenting the invariant enforced today (per-fold fuel ceiling
  and its purpose) without roadmap language."

### Liaison verification (confirmed on trunk 71c8856d7)

`fuel_budget: u64` field comment (:100-103): "…This is the interim, sync first-step toward full gas
(§22a); the budget is uniform per fold today — per-session gas accounting arrives with the async
substrate." The DURABLE invariant (per-fold fuel ceiling bounds a runaway guest — the PR#1009 DoS guard)
is good; the "interim / toward §22a / arrives with async substrate" roadmap language rots. Keep the
invariant + its purpose, drop/trim the roadmap forecast. Comment-only.

Owner: **v-agent-harness** (`cdz-kernel` wasm_host, `4782e74cc`). Reclassify Engine::new + set_fuel errors
off the guest-facing variants (InvalidComponent/Trap) onto a host-setup variant; trim the roadmap comment
to today's invariant.
