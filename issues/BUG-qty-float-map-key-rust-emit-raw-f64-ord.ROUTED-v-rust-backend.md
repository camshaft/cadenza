# BUG (rust-backend emit): a Float-valued Qty as a CHAMP Map key fails `f64: Ord` — the total-order wrapper is missing on the Qty path

**Status:** OPEN — routed to `v-rust-backend`. Found by the breaker tick-352 tri-target battery
(the periodic whole-corpus sweep; per-case gating is structurally blind to this class).

**Symptom:** `qkm1`/`qkm3` (18-units-of-measure, quantities as Map keys) run correctly on wasm but
FAIL TO BUILD on rust AND rust-async: `error[E0277]: the trait bound f64: Ord is not satisfied`.

**Discriminator (verified):** a BARE `Float64` Map key (fk1, 03-equality: the -0.0 key cell)
PASSES on rust — bare float keys get the total-order treatment. A `Qty Float64 <unit>` key exposes
the RAW `f64` as the map key type, missing that wrapper. The fix is wherever the Qty rust emit
derives its key/ord instance: route it through the same total-order wrapper bare floats use.

**Repro:** `cargo xtask gate spec/semantics/18-units-of-measure.sexp --case qkm1 --target rust`.

**Meanwhile:** qkm1/qkm3 pinned as tracked known-FAIL rows in both rust baselines (the #4547
mechanism) — they flip to pass when the wrapper lands; wasm rows unaffected.
