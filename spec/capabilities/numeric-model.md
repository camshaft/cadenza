# Capability — Numeric Model

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines numeric behavior: exactness, conversions, promotion, and overflow. Requirements
> realize [Core Principle III](../../constitution.md) and [Core Principle VII](../../constitution.md)
> and trace to [overview §4](../overview.md) and [overview §5](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading. The concrete widths, representations, and modes are
> pinned at the declared-default location.

## Purpose And Scope

This capability fixes how numbers behave: that different numeric types do not silently promote into
one another, that integer overflow has a defined result, that arithmetic declared exact loses no
precision, and that floating-point results are deterministic. It states the behavior; the concrete
widths and byte forms are the declared numeric-model default, and the floating-point emission
constraints are the determinism contract.

## No Silent Promotion

### Numeric Types Do Not Silently Promote

An operation on two numeric values of different types MUST require an explicit conversion rather than promote one operand implicitly.

The type of an arithmetic result MUST be determined by the operand types and the operation, not by an implicit widening the author did not write.

## Integer Widths

### An Integer Type Is Indexed By A Compile-Time Width

An integer type MUST be identified by a signedness and a bit width, so that two integer types of different width or signedness are distinct types that do not silently convert to one another.

The bit width of an integer type MUST be resolved from a compile-time value and MUST NOT be determined by runtime data, so that an integer's width is fixed before the program runs rather than dependent on a value computed at runtime.

A bit width that is outside the range the numeric model admits MUST be rejected at compile time with the machine-readable diagnostic for the unsatisfied width constraint, rather than accepted or trapped at runtime.

### A Conversion Between Integer Types Is Explicit

A conversion between two integer types MUST be written explicitly, as either a range-checked conversion that traps on a value outside the target type's range or a truncating conversion that keeps the target type's low bits, never an implicit widening or narrowing.

## Overflow

### Overflow Is Defined

An integer operation that overflows its type MUST have a defined, deterministic outcome fixed by the numeric model, whether that outcome is a value or a trap.

The compiler MUST NOT emit an integer operation whose overflow behavior is undefined.

## Exactness

### Exact Arithmetic Is Exact

An operation on values of a numeric type declared exact MUST NOT lose precision.

A conversion that cannot preserve a value's exactness MUST be written explicitly rather than performed implicitly.

### An Exact Rational Has A Canonical Normalized Form

An exact rational value MUST be maintained in a canonical normalized form, so that two rationals denoting the same number have one representation and one canonical byte form.

The normalization of an exact rational MUST reduce it to lowest terms and fix the placement of its sign, so that its canonical form is a deterministic function of the number it denotes rather than of how it was constructed.

### A Rational With A Zero Denominator Is Not A Value

Constructing an exact rational with a zero denominator MUST fail at a defined point rather than produce a value, because a rational with a zero denominator denotes no number.

## Floating-Point

### Floating-Point Follows The Determinism Contract

A floating-point operation MUST produce a result consistent with the emission constraints in the determinism-and-fuel contract.

A floating-point value MUST serialize under the canonical form fixed by the deterministic-value-form contract.
