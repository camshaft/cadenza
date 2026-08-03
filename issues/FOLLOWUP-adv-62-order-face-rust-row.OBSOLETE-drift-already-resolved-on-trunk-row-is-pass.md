# FOLLOW-UP: flip the adv-62 order-face rust baseline row todo→pass

My adv-62 order-face pin ("two DISTINCT let-bound host calls each captured by its own escaping
closure fire once each in order (adv-62)") was pinned with a `todo` row on .gate-baseline-rust
(rust DECLINED the closure-in-tuple-through-host shape at pin time). A peer's rust-effects fix has
since made it PASS on rust — `gate --check --target rust` reports it as "1 newly passing".

ACTION (next tick, own small MR): flip that ONE row in .gate-baseline-rust from `todo` to `pass`
(edit the single line, don't `gate --save` the whole file — see the rust-baseline reorder trap).
Verify: `cargo xtask gate --target rust --case "two DISTINCT let-bound host calls"` → PASS, then
`gate --check --target rust` → 0 newly-passing. Keep the wasm/rust-async rows as-is unless they also
flipped (check first). Do NOT bundle with an unrelated pin.
