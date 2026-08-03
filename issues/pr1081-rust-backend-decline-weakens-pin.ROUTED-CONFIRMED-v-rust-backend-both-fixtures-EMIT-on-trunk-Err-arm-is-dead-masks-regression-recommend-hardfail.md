# PR #1081 review comment — rcdzc/src/backend/rust/tests.rs (v-rust-backend)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1081
(PR: "cand: v-rust-backend — backend/rust/tests.rs (#2)").

## DECLINE-tolerant test weakens the regression pin + 4× recompile (Copilot, backend/rust/tests.rs:6823) — test-coverage/efficiency
> The test currently treats a Rust-backend DECLINE as acceptable (`Err(_) => {}`), which means it
> will still pass even if the backend stops emitting for `compare` over `(List (Option Int64))` (and
> even though this type/operation should be representable). That makes the regression pin much
> weaker and can mask real breakages. Also, inside the `Ok(_)` arm it recompiles the same source
> (`compile_rust_result` then `compile_rust`) and runs `rustc` 4 times; you can compile once and
> assert all 4 probes in a single round-trip.

Two points: (1) if the type/op IS representable on the rust backend, the `Err(_) => {}` arm should
be a hard failure (or the known-decline reason asserted), not silently accepted — otherwise the pin
can't catch an emit regression; (2) compile-once and assert the 4 probes to cut 4 rustc invocations
to 1. (Note the v-rust-backend memory: empty-collection E0282 declines on rust while wasm runs can be
legit — so confirm this specific type is genuinely expected to emit before hard-failing the arm.)
