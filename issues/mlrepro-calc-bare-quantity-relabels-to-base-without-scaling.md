# Calculator: a bare non-base quantity relabels to the base unit WITHOUT applying the conversion factor

**Reported by operator, 2026-07-15, via concierge.** (Operator saw it as "truncating trailing 0s"
— the real cause is more general; see diagnosis.)

## Symptom the operator saw
- `5 kilometer` prints `5 meter`
- `5 kilometer + 5 meter` prints `5005 meter`

## Full repro (via `cdz-calc --once "<expr>"`, default exact/ML surface)

| input | prints | correct? |
|---|---|---|
| `5 kilometer` | `5 meter` | ❌ should be `5000 meter` (or `5 kilometer`) |
| `1 kilometer` | `1 meter` | ❌ should be `1000 meter` |
| `5 mile` | `5 meter` | ❌ label AND scale wrong |
| `5 foot` | `5 meter` | ❌ label AND scale wrong |
| `2 kilometer + 3 kilometer` | `5 meter` | ❌ should be `5000 meter` |
| `5 kilometer + 0 meter` | `5000 meter` | ✅ (mixing forces normalize-to-base) |
| `5 kilometer + 5 meter` | `5005 meter` | ✅ |
| `5000 meter` | `5000 meter` | ✅ (already base) |
| `5 kilometer * 1` | `5` | ❌ unit dropped entirely |

## Diagnosis (NOT trailing-zero truncation)

The coefficient is never truncated. The bug is that a quantity in a **non-base unit**, when
rendered on its own (or combined with same-unit operands), gets its **unit label rewritten to the
base unit (`meter`) but its numeric coefficient is NOT multiplied by the conversion factor**. So
`5 kilometer` becomes `5 meter` (relabel, no ×1000 scale). Every non-base LENGTH unit collapses to
`meter` with the coefficient unchanged (`5 mile` → `5 meter`, `5 foot` → `5 meter`).

The **mixed-unit path is correct**: `5 kilometer + 0 meter` → `5000 meter`. Adding a base-unit
operand forces a real normalize-to-base that *does* apply the ×1000 factor. That asymmetry is the
tell: the conversion factor is applied on the binary-op/coercion path but MISSING on the
single-quantity display/normalization path.

`5 kilometer * 1` → `5` drops the unit entirely — likely a separate but adjacent
scalar-multiply-loses-unit issue; worth checking in the same pass.

## Likely area
The calculator's value **display / normalization** of a `Qty` whose `Unit` carries a non-unit
conversion factor. Wherever a bare `Qty` is rendered, it appears to substitute the base unit's
NAME without running the same `UnitConversion` scaling the `+`/coercion path uses. Unit vocabulary
+ conversion factors live in `rcdzc/src/prelude.rs` `unit_families` (see
[[unit-family-plural-aliases]]); the calc surface is `implementation/seed/crates/cdz-calc/`
(`lib.rs`/`runtime.rs`) and the shared quantity display. Make the single-quantity render reuse the
same normalize-to-base-with-factor routine the mixed path already uses (one source of truth).

## Acceptance
- `5 kilometer` → `5000 meter` (or `5 kilometer`, whichever the calc's display contract specifies —
  but the NUMBER and UNIT must agree; today they don't).
- `2 kilometer + 3 kilometer` → `5000 meter`; `5 mile`/`5 foot` render with the correct scaled
  value and a consistent label.
- `5 kilometer * 1` keeps its unit.
- Existing correct mixed cases (`5 kilometer + 5 meter` → `5005 meter`) still pass.
- Add corpus/calc regression cases for a bare prefixed quantity and a homogeneous-prefix sum.
