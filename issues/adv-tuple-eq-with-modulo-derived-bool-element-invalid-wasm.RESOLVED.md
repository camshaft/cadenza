# MISCOMPILE (wasm-only, v-property-testing): structural tuple `=` with a `%`-derived Bool element → invalid wasm

**Severity:** correctness — cdz compile --target wasm SUCCEEDS but cdz-run/wasmtime REJECTS at instantiation
(`wasm[0]::function[6]`). Rust backend emits fine (backend disagreement).

## Minimal repro
`(do (def (main (: s Int64)) (= (tuple 5 (= (% s 2) 0)) (tuple 5 (= (% s 2) 0)))) (export main))`

## Characterization (v-property-testing, precise)
- Trigger = [structural tuple `=`] × [a Bool element] × [that Bool computed via `%` (modulo/remainder)].
- `(tuple 5 (< s 3))` compared → OK. `(tuple 5 (= (+ s 1) 6))` → OK. `(tuple 5 (= (& s 1) 0))` → OK. Only `%` breaks it.
- `(= (% s 2) 0)` as a SCALAR (not in a compared tuple) → OK. `(tuple 5 true)` compared → OK. `(tuple s s)` compared → OK.
- Likely: the synthesized per-element compare mis-emits the `%` (i64.rem_s?) or a valtype/packed-bool mismatch
  when the Bool subexpression contains a remainder op.

## Seam
wasm backend structural-equality emission (backend/wasm/select.rs, the compound-`=` compare-function synthesis)
× the `%`/Int64.rem lowering in that nested context. Runtime tuple/record/set/map `=` otherwise works — narrow
miscompile within the working tuple-`=` path.

Graded case Fails-wasm. v-property-testing has a `<`-based workaround for its own corpus witness (unblocked).

## SHARPENED 2026-07-16 (v-property-testing): it's CONST-DIVISOR div/rem, not modulo-specific
FAILS (invalid wasm): (% s 2) AND (/ s 2) — checked DIV or REM by a CONSTANT divisor — feeding a Bool in a compared tuple.
WORKS: (% s s) non-constant divisor (runtime zero-check lowering) → OK; %-result as the INT element (not bool) → OK;
  non-div bool subexprs → OK; the same bool as a SCALAR (no tuple) → OK.
PRECISE TRIGGER = [checked DIV or REM by a CONSTANT divisor] → [Bool] → [element of a structurally-=-compared tuple].
So the CONST-DIVISOR optimized div/rem form is mis-emitted specifically inside the SYNTHESIZED per-element compare
function of a compound `=` (not in normal expression position, where it's fine). Seam narrows to: compound-= compare-fn
synthesis × the const-divisor div/rem lowering. wasm-only; rust fine.

## DIAGNOSIS (v-runtime, 2026-07-16) — slot-TYPE collision, NOT a rem mis-emit; localized, fix pending
Reproduced on trunk@40a57b74c. `wasm-tools validate` → `func 6 (main) failed: type mismatch: expected
i32, found i64 (at offset 0x206)`. func 6 = MAIN (funcs 0-5 are the runtime imports), NOT a synthesized
compare fn — the two tuples + `value-eq` are built inline in main.

func 6 decl: `(param i64) (local i64 i32 i32 i32)` = locals 0=s(i64),1(i64),2(i32),3(i32),4(i32). The
signed-const-power-of-two rem sequence (emit_div_rem, select.rs ~12637) tee's its DIVIDEND scratch into
**local 2** and reads it as i64 (`local.tee 2; local.get 2; i64.const 63; i64.shr_s; …`) — but local 2 is
declared **i32**. So slot 2 is claimed by TWO producers with different types: the rem dividend (i64) AND
something i32 (the packed-bool element result / a tuple-build handle slot), and the scratch_ty→declared
merge keeps i32 → the i64 tee is invalid.

ROOT: a slot-index aliasing between `emit_div_rem`'s dividend scratch and the enclosing tuple-element /
value-eq slot accounting — NOT the rem arithmetic (it's value-correct in isolation; only the slot TYPE
collides). ⚠ First fix attempt (change emit_div_rem `sa = base` → `sa = *high`) did NOT resolve it — the
collision persists at slot 2, so `*high` there already equals the bool-element's slot. The real fix needs
tracing WHY the div/rem dividend slot and the bool-element slot coincide: likely the `Core::Tuple` emit's
per-element `elem_base = *high` is NOT bumped past a scalar element's transient scratch before the next
element, OR emit_div_rem's `scratch_ty.insert` races the bool result's slot. NEXT: trace the Tuple emit's
base/high threading for `(tuple 5 <bool-with-rem>)` — instrument the slot each of {rem-dividend,
bool-result, element-handle} gets. wasm-only (rust backend fine). Fix is in select.rs slot allocation.

## UPDATE 2 (v-runtime, 2026-07-16) — RULED OUT emit_div_rem slot; the tee is elsewhere
Probes (trunk@42099b395): `emit_div_rem` signed-pow2 REM path fires with `sa(dividend)=base=2, *high→3, i64`.
Changed `sa = base` → `sa = *high` (fresh slot, permanently bumped, operand floats above) — did NOT fix
it (still `expected i32, found i64 @ 0x206`). So the colliding `local.tee 2` (i64 value into i32-declared
slot 2) is NOT emit_div_rem's `local.set sa`. KEY DISASM DETAIL: the fault instruction is `local.TEE 2`
(not set) at the START of the bias sequence — `local.tee 2; local.get 2; local.get 2; i64.const 63;
shr_s; i64.const 63; shr_u; add; i64.const 1; shr_s; i64.const 1; shl; sub; local.set 1`. That is the REM
result computed into slot 1, but the DIVIDEND is tee'd into slot 2 by a DIFFERENT emit than emit_div_rem
(which uses local.SET, and whose sa I moved off slot 2 with no effect). HYPOTHESIS for next tick: the
`local.tee 2` dividend comes from `emit_operand`/`emit_scalar` teeing the operand into a slot the ENCLOSING
tuple/value-eq context declared i32 — i.e. the collision is at the OPERAND-emit or bool-materialization
layer, not the div/rem body. Also note: `(= (% s 2) 0)` should hit the `rem_pow2_mask` EVEN-TEST peephole
(`s & 1; eqz`, no dividend tee) — the disasm shows BOTH a bias-sequence rem AND `s&1;eqz`, so the peephole
and the full div/rem may BOTH emit (one dead), and the dead one's tee lands in an i32 slot. NEXT: find
what emits `local.tee 2` for the dividend (grep LocalTee near div/rem/rem_pow2/operand-emit); check if the
even-test peephole and emit_div_rem both run for the same node.
