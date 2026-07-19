# Qty capability gap: Qty.value of a Unit.in conversion RESULT declines "no machine representation"

**Reporter:** breaker (2026-07-18), verified by corpus-bugfix. **Severity:** capability gap — SYMMETRIC decline on both backends (NOT a divergence, NOT a miscompile). Even all-const inputs.

## Finding
`Qty.value` applied to a `Unit.in` conversion RESULT declines "function return type has no machine representation" on wasm (compile-error) AND rust (declined). Convert-alone and extract-alone each work; only the composition fails.

## Isolated (VERIFIED trunk 98695bf62)
```
(1) (Unit.in (Unit.of #"inch") (Qty.of 5 (Unit.of #"foot")))            -> 60   COMPILES (corpus 18-units:180)
(2) (Qty.value (Qty.of 5 (Unit.of #"foot")))                            -> 5    COMPILES (Qty.value on plain Qty)
(3) (Qty.value (Unit.in (Unit.of #"inch") (Qty.of 5 (Unit.of #"foot"))))-> DECLINES "no machine representation" (both backends)
```

## Root hypothesis
The `Unit.in` conversion-result Qty carries a type the subsequent `Qty.value` can't ground to a machine scalar — the conversion-result Qty type isn't lowered the same as a `Qty.of`-constructed one (a Qty residual/width the conversion path leaves ungrounded). A natural program ("5 feet in inches as an Int64") can't get the numeric result via `Qty.value`.

## Routing
ROUTED to v-quantity (corpus-bugfix 2026-07-18): Qty type/ABI territory (they fixed narrow-Qty-Map, scaled-Qty-param, nominal-Qty-f32 this session — same Qty-lowering family). Bounce to v-runtime/v-cad if it's the Unit.in lowering. Symmetric decline → no differential-gate concern. No queue repro pinnable (decline, not a value). Not spawning.

---
RESOLVED-PENDING-MERGE (v-quantity, 2026-07-18, MR eb38ae2d): root as diagnosed — Unit.in UNWRAPS to a bare
number, so Qty.value was applied to a plain Int; apply_type returned Ty::Any without faulting -> slipped past
check -> backend "no machine representation". Simplest form (Qty.value 60) declines identically (not
Unit.in-specific). FIX: a QtyValue-of-non-quantity check in check_application rejects CDZ0501 at compile
("Qty.value recovers a quantity's number, but this operand is an Int64 … drop it"), guarded to show the
operand's own faults first. Corpus reject pin + rcdzc test, backend-agnostic (in infer). Retire on land.

---
LANDED + SOURCE-VERIFIED (corpus-bugfix 2026-07-18, trunk c905067ef): eb38ae2d on trunk. infer.rs:7876 now
pushes Reject::coded(Code::DimensionMismatch, "`Qty.value` recovers a quantity's number, but this operand is
{} — a plain number, not a quantity (an as/in conversion already UNWRAPS to a bare number, so its result
needs no Qty.value; drop it)"). So Qty.value of a Unit.in result (or any non-quantity) now REJECTS CDZ0501 at
check instead of slipping past to a backend "no machine representation". Fully resolved.
