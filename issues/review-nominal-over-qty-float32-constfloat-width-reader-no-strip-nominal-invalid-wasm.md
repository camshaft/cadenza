# REVIEWER FINDING 2026-07-18 — nominal-over-Qty-Float32 map value emits INVALID WASM (ConstFloat width readers peel raw Ty::Qty but not strip_nominal)

> STATUS UPDATE 2026-07-18: **RUST face FIXED** by v-rust-backend `6463e034f` (new `float_width_of` =
> strip→peel→strip, both const-float arms; held behind their S3 MR). The rust E0308 twin was reproduced +
> closed. **WASM face STILL OPEN** — `select.rs peel_qty_ty` still peels raw Qty with no strip_nominal
> (the face my `wasmparser::validate` probe actually caught). v-rust-backend routed the wasm fix to the
> select.rs float-const owner (v-quantity, `c6923a2b6` author); reviewer also noted v-quantity directly.
> Fix = same strip→peel→strip on `peel_qty_ty` / its ConstFloat call sites. Promote this repro when the
> wasm fix lands (wasm validates).

Post-merge review of **c6923a2b6** ("rcdzc: peel Ty::Qty in the ConstFloat width readers — fixes
Float32-Qty heap-value miscompile"). That fix closed the BARE `(Qty Float32)` case, but MISSED the
NOMINAL-over-Qty-Float32 wrapper — the exact structural gap v-rust-backend already found + fixed for the
INTEGER `int_ty_of` in `1afc93bb2` (which was upgraded to `strip_nominal → peel Qty → strip_nominal`).
This ConstFloat fix reintroduced the PRE-upgrade simple-peel.

## Root cause

Both `peel_qty` (rust `backend/rust/expr.rs:3299`) and `peel_qty_ty` (wasm `backend/wasm/select.rs:14359`)
peel only a RAW `Ty::Qty`:
```
fn peel_qty(ty: Ty) -> Ty { match ty { Ty::Qty { inner, .. } => *inner, other => other } }
```
The `Core::ConstFloat` / `Core::ConstFloatNan` width readers call `peel_qty(type_of(id))` with NO leading
`strip_nominal`. So a NOMINAL-over-Qty-Float32 — `(type Len (Q (Qty Float32 meter)))` stored as a heap
value — is `Ty::Nominal { inner: Qty }`, misses the raw-Qty peel, and defaults to f64:
- wasm: emits `f64.const` where `box-float32` wants f32 → INVALID MODULE (`type mismatch: expected f32,
  found f64`).
- rust: almost certainly the E0308 twin (`f64::from_bits` into an f32 map slot) — same `peel_qty` shape as
  the confirmed integer nominal-over-Qty (`1afc93bb2`); not independently reproduced here but structurally
  identical.
`cdz check` passes it (a check-vs-link gap).

## Reproducer (VERIFIED — wasm INVALID)

Probed via a temporary test (`compile_component` + `wasmparser::validate`, reverted, worktree clean):
```
(module m (type Len (Q (Qty Float32 (Unit.base #"meter"))))
  (def (main) ((. Qty value)
    (match ((. Map lookup)
             ((. Map insert) ((. Map empty)) 1
              (Len.Q ((. Qty of) ((. Float32 of) 1.5) ((. Unit base) #"meter")))) 1)
      ((Some (Len.Q q)) q)
      ((None) ((. Qty of) ((. Float32 of) 0.0) ((. Unit base) #"meter")))))) (export main))
```
→ `PROBE nominal-over-Qty-f32: INVALID WASM → type mismatch: expected f32, found f64 (at offset 0x350)`.
CONTROL (bare `(Qty Float32)` map value, the fix's own case) → VALID.

## Fix

Give the ConstFloat/ConstFloatNan width readers (both backends) the SAME strip→peel→strip the integer
`int_ty_of` got in `1afc93bb2` — either make `peel_qty`/`peel_qty_ty` do `strip_nominal` before and after
the Qty peel, or `strip_nominal()` at the call sites around `peel_qty`. Mirrors the integer lockstep. Add a
nominal-over-Qty-Float32 map-value pin (wasm validates + rust runs to 1.5).

## Severity

INVALID-WASM emit (wasm) / likely E0308 (rust) — a check-vs-link miscompile on the value-materialization
path, EXOTIC reachability (a nominal newtype wrapping a Float32 quantity, stored as a heap value). Same
family as the closed integer nominal-over-Qty. Owner: **v-rust-backend** (owns the `expr.rs`/`select.rs`
width readers + authored the integer twin's fix). Routed as a note to them + this queue item.

---
ROUTED to v-rust-backend (reviewer noted them directly; corpus-bugfix confirmed+tracking 2026-07-18).
VERIFIED trunk a6c136526: cdz run -> "invalid component ... wasm[0]::function[11]" (f64.const into f32 box).
ROOT: ConstFloat width readers peel_qty (rust expr.rs:3299) / peel_qty_ty (wasm select.rs:14359) peel a RAW
Ty::Qty with NO strip_nominal, so Ty::Nominal{inner:Qty} misses -> f64 default. EXACT gap v-rust-backend
fixed for integer int_ty_of in 1afc93bb2 (strip->peel->strip); ConstFloat fix c6923a2b6 reintroduced the
simple peel. FIX: strip_nominal->peel_qty->strip_nominal on ConstFloat/ConstFloatNan readers, both backends.
Rust E0308 twin near-certain. Not spawning (their family). Promote when fixed.
