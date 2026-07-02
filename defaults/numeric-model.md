# Numeric Model — Declared Default

> **What this file is.** The concrete numeric widths, representations, and modes that realize the
> numeric *behavior* the specification states technology-neutrally (numeric-model.md capability;
> deterministic-value-form.md §"Numeric Serialization"; determinism-and-fuel.md §"Floating-Point
> Emission Is Determinism-Constrained"). The spec fixes that numeric types do not silently promote,
> that overflow is defined, that exact arithmetic is exact, and that floats are deterministic; this
> file pins the concrete choices.
>
> This is a **declared default**. Because these choices fix bytes that cross the boundary and enter
> the canonical value form, a change to them is an ABI-level change under the constitution's
> Governance Floors.

## The default choices

| Concern | Default |
|---|---|
| Default integer | **checked signed 64-bit**; overflow **traps** deterministically rather than wrapping |
| Wrapping integers | available as a **distinct type**, not a mode on the default integer, so wrap is never silent |
| Unsigned integers | available as distinct fixed-width types (8/16/32/64-bit) |
| Arbitrary-precision integer | a distinct **big-integer** type, opted into explicitly |
| Floating-point | **IEEE-754 binary64**, single fixed rounding mode (round-to-nearest-even), no fused-multiply-add contraction, canonical not-a-number bit pattern |
| Exact rational | a **normalized pair of big-integers** (reduced to lowest terms, fixed sign convention), opted into explicitly |
| Numeric promotion | **none** — an operation on two different numeric types requires an explicit conversion |

## Why these choices, against the north star

- **No silent promotion** (the sharpest departure from earlier Cadenza, where integer division
  yielded a rational): a surprising inferred type is a cost to both agent-writability and
  verification, and a hidden coercion path is a cost to reproducibility. Every conversion is
  explicit in the source, so an agent and a verifier both see exactly which arithmetic happens.
- **Checked-and-trapping default integer:** overflow is a defined, deterministic event, not
  undefined behavior and not a silent wrap; a program that needs wrap says so with a distinct type.
- **Deterministic float mode:** fixed rounding, no FMA contraction, and a canonical NaN make
  floating-point results byte-identical across conforming runtimes, as the determinism contract
  requires.
- **Exactness where declared:** big-integer and rational are exact and opted into explicitly, so a
  program pays their cost only when it asks for exactness.

## Relationship to units of measure

Units of measure, if the optional dimensional-analysis capability is included, attach to numeric
values as a compile-time-only layer and are erased before emission (units-of-measure.md). They do
not change the numeric byte forms pinned here.
