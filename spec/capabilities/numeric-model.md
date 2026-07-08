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

### An Overflow-Fallible Operation Reports Overflow Rather Than Trapping

An integer type MUST offer, alongside its trapping arithmetic, a named overflow-fallible form of each of addition, subtraction, and multiplication whose result is the exact value wrapped in the present case when it is in range and the absent case when the operation overflows, so that a program can branch on overflow without trapping.

The overflow-fallible form MUST be opted into by name at the operation, so that an author who writes the ordinary operator still gets the trapping outcome and overflow is never silently reported.

### A Wrapping Operation Has A Defined Modular Outcome

An integer type MUST offer a named wrapping form of each of addition, subtraction, and multiplication whose result on overflow is the two's-complement value reduced modulo the type's range, so that modular arithmetic has a defined non-trapping outcome distinct from the trapping default.

The wrapping form MUST be opted into by name at the operation, so that it never displaces the trapping default an unqualified operator selects.

## Arbitrary Precision

### An Arbitrary-Precision Integer Has Unbounded Range

An arbitrary-precision integer type MUST represent every integer with no maximum or minimum bound, so that an arithmetic operation on it never overflows.

An arithmetic operation on arbitrary-precision integers MUST NOT trap for the magnitude of its result, growing its representation as the result requires rather than wrapping or trapping.

### An Arbitrary-Precision Integer Is A Distinct Type Opted Into Explicitly

An arbitrary-precision integer MUST be a distinct numeric type, so that it does not silently convert to or from a fixed-width integer without an explicit conversion.

## Default Literal Type

### A Module May Declare Its Default Integer Literal Type

A module MAY declare, through a module directive (modules-and-namespaces.md §"A Module Directive Is Drawn From A Fixed Set"), the integer type that an integer literal with no other constraint takes within that module.

When a module declares no default integer literal type, an integer literal with no other constraint MUST take the numeric model's default integer type.

The type named by a default-integer-literal directive MUST be an integer type the numeric model admits, and a directive naming a non-integer type MUST be rejected with the machine-readable diagnostic for that unsatisfied constraint.

### A Declared Default Applies At The Definition Site

The default integer literal type in force for a literal MUST be the one declared by the module in which the literal is written, not one declared by any module that imports it, so that importing a module never changes the type its literals take.

### A Declared Default Fixes A Type, Not A Conversion

Declaring a default integer literal type MUST only determine the type an otherwise-unconstrained integer literal takes, and MUST NOT introduce any implicit conversion between numeric types, so that every no-silent-promotion rule applies unchanged to a literal whatever its declared default type.

An explicit type annotation or other constraint on an integer literal MUST take precedence over the module's declared default integer literal type.

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
