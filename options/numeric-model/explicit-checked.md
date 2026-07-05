# Numeric Model — Choice: explicit-checked

> **The default choice for the `numeric-model` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It names the concrete numeric widths,
> representations, and modes that realize the numeric behavior the specification states
> technology-neutrally (numeric-model.md capability; deterministic-value-form.md §"Numeric
> Serialization"; determinism-and-fuel.md §"Floating-Point Emission Is Determinism-Constrained").
>
> Because this choice fixes bytes that cross the boundary and enter the canonical value form, a
> change to it is an ABI-level change under the constitution's Governance Floors.

## The default choices

| Concern | Default |
|---|---|
| Default integer | **checked signed 64-bit** (`Int64`); overflow **traps** deterministically rather than wrapping. It is the type a bare integer literal takes when nothing constrains it otherwise. |
| Fixed-width signed integers | `Int8`, `Int16`, `Int32`, `Int64` — distinct **checked** two's-complement types; each traps on overflow of its own range, none silently converts to another width |
| Fixed-width unsigned integers | `UInt8`, `UInt16`, `UInt32`, `UInt64` — distinct **checked** types over `0..=2ⁿ−1`; each traps on overflow or on a negative result, none silently converts to another width |
| Integer conversions | **explicit** only: `T.of x` is a **checked** conversion (traps when `x` is outside `T`'s range), `T.wrap x` is a **truncating/wrapping** conversion (keeps the low bits under `T`'s two's-complement representation). Neither is ever performed implicitly. |
| Wrapping integers | available as a **distinct type**, not a mode on the default integer, so wrap is never silent |
| Arbitrary-precision integer | a distinct **big-integer** type, opted into explicitly |
| Floating-point | **IEEE-754 binary64**, single fixed rounding mode (round-to-nearest-even), no fused-multiply-add contraction, canonical not-a-number bit pattern |
| Exact rational | a **normalized pair of big-integers** (reduced to lowest terms, fixed sign convention), opted into explicitly |
| Numeric promotion | **none** — an operation on two different numeric types (including two different integer widths or signednesses) requires an explicit conversion |

## The fixed-width integer family

The default integer set is the symmetric eight-type family `{Int, UInt} × {8, 16, 32, 64}`. Every one
is a distinct, checked type: an operation whose result leaves the type's range traps (`"integer
overflow"`, numeric-model.md §"Overflow Is Defined"), exactly as the default `Int64` does — no width
wraps silently, and no signedness is silently reinterpreted. `Int64` remains the default a bare literal
takes; the other seven are reached by an annotation (`(: 200 UInt8)`), a per-width bound
(`UInt8.max` = 255, `Int8.min` = −128), or a conversion.

**Representation.** Each width is stored in the smallest core-wasm integer that holds it and kept in its
canonical range: `Int8/16/32` and `UInt8/16/32` compute in an `i32` (masked / sign-extended to the
width after each operation), `Int64`/`UInt64` in an `i64`. Signedness selects the machine operation —
`div_s`/`div_u`, `lt_s`/`lt_u`, `shr_s` (arithmetic, sign-filling) vs `shr_u` (logical, zero-filling) —
and the overflow bounds. `UInt64` can hold values above `Int64.max`; its comparisons and right shift are
unsigned, so `(< (: 0 UInt64) UInt64.max)` is true where the same bit pattern read as `Int64` would be
negative.

**Conversions are explicit and total-or-defined.** `T.of x` checks and traps when `x` does not fit `T`;
`T.wrap x` keeps the low bits (`UInt8.wrap 256` = 0, `Int8.wrap 255` = −1). The seed's `Int.to-byte`
(truncate to `0..=255`) is exactly `UInt8.wrap`, and `Float64.of-int` is the float analogue of `T.of`;
this family systematizes both under one naming rule rather than adding ad-hoc per-target primitives.

**Boundary mapping (ABI-level).** The eight types map one-to-one onto the component model's fixed-width
integer primitives — `Int8→s8`, `UInt8→u8`, `Int16→s16`, `UInt16→u16`, `Int32→s32`, `UInt32→u32`,
`Int64→s64` (unchanged), `UInt64→u64`. Because this fixes bytes that cross the boundary and enter the
canonical value form, adding these mappings is an **additive** change under the constitution's
Governance Floors and the deterministic-value-form contract's §"Additive Evolution" (no
already-serializable value changes form — `Int64` keeps `s64`), but it still touches the **frozen**
component-abi.md and MUST land through the coordinated governance act with a version increment, not as an
incidental edit. The rows to add to component-abi.md §"Every Exported Type Has A Stable Boundary
Representation" are the seven new width→primitive pairs above.

## Why these choices, against the north star

- **No silent promotion** (the sharpest departure from earlier Cadenza, where integer division
  yielded a rational): a surprising inferred type is a cost to both agent-writability and
  verification, and a hidden coercion path is a cost to reproducibility. Every conversion is
  explicit in the source, so an agent and a verifier both see exactly which arithmetic happens.
- **Checked-and-trapping default integer:** overflow is a defined, deterministic event, not
  undefined behavior and not a silent wrap; a program that needs wrap says so with a distinct type.
- **A full fixed-width family, not just `Int64`:** the first Cadenza artifact is a *compiler*, and a
  compiler is the sharpest witness that one width does not fit — it manipulates `UInt8` (the bytes of a
  wasm module) and `UInt32` (section sizes, LEB128 operands, table and memory indices) constantly.
  Realizing only `Int64` forces that code to carry untyped 64-bit integers where it means a byte or a
  32-bit index and to mask by hand, which discards exactly the static typing the language promises. The
  symmetric `{Int, UInt} × {8, 16, 32, 64}` set is also the one the component-model boundary already
  supplies as primitives, so no width crosses the boundary by an encoding the ABI does not already fix.
- **Deterministic float mode:** fixed rounding, no FMA contraction, and a canonical NaN make
  floating-point results byte-identical across conforming runtimes, as the determinism contract
  requires.
- **Exactness where declared:** big-integer and rational are exact and opted into explicitly, so a
  program pays their cost only when it asks for exactness.

## Relationship to units of measure

Units of measure, if the optional dimensional-analysis capability is included, attach to numeric
values as a compile-time-only layer and are erased before emission (units-of-measure.md). They do
not change the numeric byte forms pinned here.
