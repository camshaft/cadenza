# pr646 — wasm scalar_box extend keys on `ground_width() < 64`, not the i32 machine slot (33..63 mid-widths)

Mirrored from GitHub PR #646 review comment (Copilot), id 3611205338.
PR: https://github.com/camshaft/cadenza/pull/646 (8-MR publish batch)
Location: `implementation/seed/crates/rcdzc/src/backend/wasm/mod.rs:293` (`scalar_box`)

## Reviewer comment (verbatim)
> In `scalar_box`, the inner `match` arm for `Ty::Int` is missing a trailing comma before the `_ =>` arm,
> which is a Rust syntax error. Also, the `extend` should be driven by the wasm machine slot width (i32 only
> for widths ≤ 32, per `lir::int_valtype`), not `ground_width() < 64` — widths 33..63 are i64 values and
> would make the later `i64.extend_i32_*` invalid.

## Triage (grep-verified on trunk)
TWO claims — one FALSE, one PLAUSIBLY-REAL:

1. **"missing trailing comma → Rust syntax error" = FALSE POSITIVE.** The `Ty::Int(it) if ... => { ... }`
   arm ends in a `{}` BLOCK; Rust does NOT require a comma after a block match-arm. The code compiles (it's
   on trunk, gated green by pr-sync's `cargo build`). Dismiss this half.

2. **width-logic = PLAUSIBLY REAL, owner judgment needed.** `scalar_box` (mod.rs:289) fires the narrow-int
   box-extend on `Ty::Int(it) if it.ground_width() < 64`. But `int_valtype` (lir.rs:448) maps
   `ground_width() <= 32 → I32 else I64`. So a Qty-inner Int of width 33..63 (e.g. Int40) is ALREADY an i64
   machine value, yet `ground_width() < 64` is true → it'd signal the i32→i64 extend
   (`emit_box_i32_to_i64_extend` / `i64.extend_i32_*`) on an already-i64 value = INVALID wasm. The guard
   should key on the machine slot (`int_valtype(it) == I32`, i.e. width ≤ 32), not `< 64`.
   REACHABILITY (the open question, owner call): `IntTy::fixed(signed, width)` accepts an arbitrary `u32`
   width, so a 33..63-width Int is CONSTRUCTIBLE — but is a Qty over a mid-width (33..63) Int inner actually
   reachable through this scalar-erased-Qty-result box path? If the width lattice in practice is only
   8/16/32/64 (+ narrow unusual <32), it may be latent-not-live. Same "verify the emit locus / width helper
   lockstep" family as [[int-ty-of-missing-strip-nominal-narrow-newtype-literal-box-invalid-wasm]].

## Owner
`rcdzc/src/backend/wasm/mod.rs` scalar-box/extend = v-wasm-opt (wasm backend output). Fix (if reachable):
gate the extend on `int_valtype(it) == I32` (width ≤ 32), not `ground_width() < 64`. The syntax-error half
is a hallucination — do NOT act on it.
