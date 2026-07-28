# PR#743 review comments — wasm Float32 branch grounding matches Ty::Float directly, misses Nominal/Qty-wrapped Float32

Mirrored from GitHub PR review comments (Copilot), ids `3623671049`, `3623671081`.
PR: https://github.com/camshaft/cadenza/pull/743 (merged; fix still belongs on trunk)
Locations:
- `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:5340` (tail-position `if` branch emit)
- `implementation/seed/crates/rcdzc/src/backend/wasm/select.rs:14020` (`emit_branch`)

## Comments (verbatim)

- (id 3623671049, :5340) "In the tail-position `if` branch emission, the Float32 grounding check only
  matches `Ty::Float` directly. If the `if` result is wrapped (e.g. `Ty::Nominal` or `Ty::Qty` over
  `Float32`), `valtype_of(&result)` is still `F32` (so the block type is `f32`), but this clause won't
  fire and a bare `ConstFloat` branch can still emit `f64.const`, producing an invalid module."
- (id 3623671081, :14020) "`emit_branch` has the same issue as the tail-position `if` grounding: it
  only recognizes a Float32 result when `result` is exactly `Ty::Float`, but the block/result type is
  determined by `valtype_of`, which strips `Nominal`/`Qty`. A wrapped Float32 result could still cause
  a bare `ConstFloat` arm to emit an `f64` constant into an `f32`-typed block."

## Liaison verification (CONFIRMED structural; reachability needs owner repro)

- Both grounding sites match `Ty::Float(rft)` DIRECTLY:
  - `select.rs:5327-5330` (tail-if): `else if let Core::ConstFloat(d) = core_of(db, b) && let Ty::Float(rft) = &result && rft.ground_width() == 32`
  - `select.rs:14013-14015` (emit_branch): `if let (Ty::Float(rft), Core::ConstFloat(d)) = (result, core_of(db, id)) && rft.ground_width() == 32`
- `valtype_of` (lir.rs:363) READS THROUGH the wrappers to the inner type:
  - `Ty::Nominal { inner, .. } => valtype_of(inner)` (lir.rs:383)
  - `Ty::Qty { inner, .. } => valtype_of(inner)` (lir.rs:440)
  So a `(type UserFloat (Mk Float32))` or `(Qty Float32 …)` result → block type `f32`, but the direct
  `Ty::Float` match fails → the bare `ConstFloat` branch falls through to its default `f64.const` emit →
  `expected f32, found f64` INVALID MODULE.

Reachability caveat: the `Ty::Qty` arm comment says "a `Ty::Qty` should not survive to selection"
(lower strips Qty), and lower may also erase nominals before selection — so a wrapped-Float32 result
may not actually reach these sites. But the SIBLING int-grounding clause and other select.rs sites use
`.strip_nominal()`, and this float clause does not, so the asymmetry is a latent hazard even if not
currently reachable.

Fix (per Copilot): match `result.strip_nominal()` (and strip Qty) before the `Ty::Float(rft)` /
`ground_width()==32` check, mirroring the int-grounding sibling. Owner: v-inference (emit-type-selection
lane; this float grounding just landed as `ea2be74b5` "ground a bare-ConstFloat if-branch to the if's
Float32 result width"). Routed as a note flagged POTENTIAL-CORRECTNESS — repro with
`(: (if c 1.5 0.25) UserF32)` where `UserF32` is a nominal/Qty over Float32; if it emits invalid wasm,
add a corpus/backend pin.
