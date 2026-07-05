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
| Integer types | the two **width-indexed type constructors** `Int` and `UInt`, applied to a compile-time width `N` in `1..=64`: `(Int N)` is a checked two's-complement signed integer over `−2ⁿ⁻¹..=2ⁿ⁻¹−1`, `(UInt N)` a checked unsigned integer over `0..=2ⁿ−1`. Every application is a distinct, checked type; each traps on overflow of its own range, none silently converts to another width or signedness. |
| Default integer | **checked signed 64-bit** — `Int64`, the alias for `(Int 64)`; overflow **traps** deterministically rather than wrapping. It is the type a bare integer literal takes when nothing constrains it otherwise. |
| Named width aliases | `Int8/16/32/64` and `UInt8/16/32/64` are ordinary aliases for `(Int 8)`…`(UInt 64)` — the common widths that also have a boundary representation; they are not separate primitives. Any other in-range width — `(UInt 48)`, `(UInt 62)`, `(Int 7)` — is an equally first-class type with no alias. |
| Integer conversions | **explicit** only: `T.of x` is a **checked** conversion (traps when `x` is outside `T`'s range), `T.wrap x` is a **truncating/wrapping** conversion (keeps the low `N` bits under `T`'s two's-complement representation). Neither is ever performed implicitly. |
| Wrapping integers | available as a **distinct type**, not a mode on the default integer, so wrap is never silent |
| Arbitrary-precision integer | a distinct **big-integer** type, opted into explicitly. A width `N` **above 64** is reserved to this multi-word layer, not to the width-indexed constructors above (see §"Widths above 64 are reserved"). |
| Floating-point | **IEEE-754 binary64**, single fixed rounding mode (round-to-nearest-even), no fused-multiply-add contraction, canonical not-a-number bit pattern |
| Exact rational | a **normalized pair of big-integers** (reduced to lowest terms, fixed sign convention), opted into explicitly |
| Numeric promotion | **none** — an operation on two different numeric types (including two different integer widths or signednesses) requires an explicit conversion |

## Integers are width-indexed, not a fixed set of primitives

The integer types are the two **width-indexed type constructors** `Int` and `UInt`, each a compile-time
function from a width to a type (type-system.md §"Generics Are Type-Valued Parameters" — "A generic type
constructor … MUST be a compile-time function from types to a type, applied by ordinary application").
`Int` and `UInt` differ only in that their parameter is a compile-time **natural width** rather than a
type: `(UInt 8)` is the unsigned 8-bit integer, `(Int 32)` the signed 32-bit integer, `(UInt 48)` an
unsigned 48-bit integer, `(UInt 62)` an unsigned 62-bit integer. There is no privileged set of eight
integer primitives the compiler special-cases; `Int64` is not a built-in distinct from `(UInt 48)`, it
is the *alias* `(Int 64)`. This is the same first-class-type machinery `(Option Int64)` and `(List T)`
already use, indexed by a width instead of by a type.

Every application `(Int N)` / `(UInt N)` for `N` in `1..=64` is a distinct, checked type: an operation
whose result leaves the type's range traps (`"integer overflow"`, numeric-model.md §"Overflow Is
Defined"), exactly as the default `Int64` does — no width wraps silently, and no signedness is silently
reinterpreted. `Int64` remains the type a bare literal takes when unconstrained; any other width is
reached by an annotation (`(: 200 (UInt 8))`, `(: 200 UInt8)`), a per-width bound (`(UInt 8).max` = 255,
`(Int 8).min` = −128, `(UInt 48).max` = 281474976710655), or a conversion.

**Named aliases for the common widths.** `Int8/16/32/64` abbreviate `(Int 8)`…`(Int 64)` and
`UInt8/16/32/64` abbreviate `(UInt 8)`…`(UInt 64)`. They are ordinary definitions
(`(def UInt8 (UInt 8))`), not primitives — they exist because those widths also have a boundary
representation (below) and are the ones a program writes most. An in-range width without an alias
(`(UInt 48)`, `(Int 7)`) is an equally first-class type; it simply has no shorter name.

**The width parameter is compile-time, resolved to a concrete natural.** `N` obeys the same fence every
type parameter does — type-system.md §"Generics Are Type-Valued Parameters": "A type parameter MUST be
resolvable to a concrete type at compile time, so that a type-value never flows from runtime data into a
position that determines a type." `(UInt some-runtime-value)` is rejected exactly as any other
runtime-valued type argument is: the width is always a compile-time constant. This keeps the feature at
**indexed types over compile-time naturals**, not dependent types — no runtime value ever determines a
type.

**The width constraint is an ordinary compile-time predicate.** `N` must be a natural in `1..=64`
(type-system.md §"A Generic Constraint Is A Compile-Time Predicate Over Type-Values"). `(UInt 0)`,
`(UInt 65)`, and `(UInt -3)` fail the constraint and are rejected at compile time with the
unsatisfied-constraint diagnostic (`CDZ0302`), the same way any generic instantiation whose argument
fails its parameter's constraint is rejected — not a runtime failure. The `1..=64` bound is exactly the
range that fits a single core-wasm integer register (below); widths above 64 are reserved to the
multi-word layer (§"Widths above 64 are reserved").

**Representation is computed from `N`, not selected from a table.** A value of `(Int N)` / `(UInt N)` is
stored in the smallest core-wasm integer that holds it — an `i32` for `N ≤ 32`, an `i64` for
`33 ≤ N ≤ 64` — and kept in its canonical range by masking to the low `N` bits (unsigned) or
sign-extending from bit `N−1` (signed) after each operation, with the overflow check comparing against
`±2ⁿ⁻¹` / `2ⁿ`. Computing the mask and bounds from `N` is *simpler* than a case analysis over a fixed
width set, and it is what makes an arbitrary in-range width free rather than another primitive to
hand-write. Signedness selects the machine operation — `div_s`/`div_u`, `lt_s`/`lt_u`, `shr_s`
(arithmetic, sign-filling) vs `shr_u` (logical, zero-filling) — and the overflow bounds. A `(UInt 64)`
can hold values above `Int64.max`; its comparisons and right shift are unsigned, so
`(< (: 0 (UInt 64)) (UInt 64).max)` is true where the same bit pattern read as `(Int 64)` would be
negative.

**Conversions are explicit and total-or-defined.** `T.of x` checks and traps when `x` does not fit `T`;
`T.wrap x` keeps the low `N` bits (`(UInt 8).wrap 256` = 0, `(Int 8).wrap 255` = −1,
`(UInt 48).wrap x` keeps the low 48 bits). The seed's `Int.to-byte` (truncate to `0..=255`) is exactly
`(UInt 8).wrap`, and `Float64.of-int` is the float analogue of `T.of`; the constructor systematizes both
under one naming rule rather than adding ad-hoc per-target primitives — and the rule is uniform across
every width, aliased or not.

**Boundary mapping (ABI-level) — only the aliased widths cross.** The component model provides integer
primitives at exactly eight widths — `s8/u8/s16/u16/s32/u32/s64/u64` — so only the aliased types have a
boundary representation: `(Int 8)→s8`, `(UInt 8)→u8`, `(Int 16)→s16`, `(UInt 16)→u16`, `(Int 32)→s32`,
`(UInt 32)→u32`, `(Int 64)→s64` (unchanged), `(UInt 64)→u64`. A non-aliased width like `(UInt 48)` has
**no boundary representation**, so — per component-abi.md §"Every Exported Type Has A Stable Boundary
Representation": "A type that has no defined boundary representation MUST NOT appear in an exported or
imported signature" — it is an **internal-only** type: excellent for bit-packing and field layout inside
a program, and converted to an aliased width (`((UInt 64).of x)`) to be exported. This needs no new ABI
surface beyond the eight standard widths. Because those eight fix bytes that cross the boundary and enter
the canonical value form, adding the seven new width→primitive rows (all but `s64`) is an **additive**
change under the constitution's Governance Floors and the deterministic-value-form contract's §"Additive
Evolution" (no already-serializable value changes form — `Int64` keeps `s64`), but it still touches the
**frozen** component-abi.md and MUST land through the coordinated governance act with a version
increment, not as an incidental edit. The rows to add to component-abi.md §"Every Exported Type Has A
Stable Boundary Representation" are those seven width→primitive pairs.

## Widths above 64 are reserved

A width `N` **greater than 64** is deliberately **not** a `(UInt N)` / `(Int N)` today: it fails the
`1..=64` constraint (`CDZ0302`) exactly as `(UInt 0)` does. The `1..=64` ceiling is the range a single
core-wasm register represents; a wider fixed-size integer (`(UInt 128)`, `(UInt 256)`) needs a
multi-word representation and multi-limb arithmetic, which is the **big-integer** layer's concern, not a
register-width type's. Reserving `N > 64` rather than rejecting the *notation* keeps the door open: a
later optional increment MAY lift the ceiling and realize wide fixed-size integers as a multi-word
representation, at which point `(UInt 128)` becomes a valid type with no change to the surface syntax —
only the constraint's upper bound and the representation move. Until then, a program needing more than 64
bits uses the opt-in arbitrary-precision big-integer type.

## Why these choices, against the north star

- **No silent promotion** (the sharpest departure from earlier Cadenza, where integer division
  yielded a rational): a surprising inferred type is a cost to both agent-writability and
  verification, and a hidden coercion path is a cost to reproducibility. Every conversion is
  explicit in the source, so an agent and a verifier both see exactly which arithmetic happens.
- **Checked-and-trapping default integer:** overflow is a defined, deterministic event, not
  undefined behavior and not a silent wrap; a program that needs wrap says so with a distinct type.
- **Width-indexed integers, not a fixed set of primitives:** the first Cadenza artifact is a *compiler*,
  and a compiler is the sharpest witness that one width does not fit — it manipulates `UInt8` (the bytes
  of a wasm module) and `UInt32` (section sizes, LEB128 operands, table and memory indices) constantly.
  Realizing only `Int64` forces that code to carry untyped 64-bit integers where it means a byte or a
  32-bit index and to mask by hand, which discards exactly the static typing the language promises. But
  the deeper point is that a *fixed* family of eight primitives repeats the very limitation this language
  set out to avoid: in a language whose types are first-class values and whose type constructors are
  ordinary compile-time functions, there is no reason a width should be a privileged built-in rather than
  a parameter. Making `Int`/`UInt` width-indexed means an unusual but genuinely useful width — a `(UInt
  48)` timestamp, a `(UInt 62)` tagged pointer, a `(UInt 24)` audio sample, a `(UInt 12)` ADC reading —
  is a first-class type the compiler *computes* (its mask, bounds, and ops all follow from `N`), not a
  wrapper-plus-hand-written-operators the author maintains. This is exactly the friction a fixed
  primitive set imposes elsewhere (a wrapped `u64` with by-hand masking and per-width operator impls);
  here the general form is *simpler* to implement than eight special cases, because the arithmetic is
  already width-parametric. The eight aliased widths are the subset the component-model boundary supplies
  as primitives, so no width crosses the boundary by an encoding the ABI does not already fix; all other
  widths stay internal, where the packing wins live.
- **Deterministic float mode:** fixed rounding, no FMA contraction, and a canonical NaN make
  floating-point results byte-identical across conforming runtimes, as the determinism contract
  requires.
- **Exactness where declared:** big-integer and rational are exact and opted into explicitly, so a
  program pays their cost only when it asks for exactness.

## Relationship to units of measure

Units of measure, if the optional dimensional-analysis capability is included, attach to numeric
values as a compile-time-only layer and are erased before emission (units-of-measure.md). They do
not change the numeric byte forms pinned here.
