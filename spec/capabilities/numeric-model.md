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

### Integer Types Have Fixed Widths

The language MUST provide a family of integer types of distinct fixed widths and signedness, each a distinct type that does not silently convert to another.

A conversion between two integer types MUST be written explicitly, as either a range-checked conversion that traps on an out-of-range value or a truncating conversion that keeps the low bits, never an implicit widening or narrowing.

## Overflow

### Overflow Is Defined

An integer operation that overflows its type MUST have a defined, deterministic outcome fixed by the numeric model, whether that outcome is a value or a trap.

The compiler MUST NOT emit an integer operation whose overflow behavior is undefined.

## Exactness

### Exact Arithmetic Is Exact

An operation on values of a numeric type declared exact MUST NOT lose precision.

A conversion that cannot preserve a value's exactness MUST be written explicitly rather than performed implicitly.

## Floating-Point

### Floating-Point Follows The Determinism Contract

A floating-point operation MUST produce a result consistent with the emission constraints in the determinism-and-fuel contract.

A floating-point value MUST serialize under the canonical form fixed by the deterministic-value-form contract.
