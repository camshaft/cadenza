# Capability — Units Of Measure

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines dimensional analysis as an optional, compile-time-only verification layer.
> Requirements realize [Core Principle VII](../../constitution.md) and
> [Core Principle VIII](../../constitution.md) and trace to [overview §5](../overview.md) and
> [overview §12](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes dimensional analysis: quantities carry units, combining incompatible dimensions
is a compile-time error, and the whole apparatus is checked at compile time and erased before
emission. It is the one piece of earlier Cadenza's identity that survives the clean room, because it
directly serves verifying properties and costs nothing at runtime — but it is an optional layer over
the numeric core, never baked into it. It states the behavior of the layer, not its surface.

## Dimensional Checking

### Dimensions Are Checked Then Erased

Dimensional consistency MUST be checked at compile time.

A unit or dimension MUST NOT appear in the emitted component, being erased after checking.

### Dimensional Mismatch Is An Error

Combining quantities of incompatible dimension MUST be a compile-time error.

A combination of quantities of incompatible dimension MUST be rejected at compile time with the machine-readable diagnostic for the unsatisfied dimensional constraint, rather than accepted or deferred to runtime.

An operation that derives a dimension MUST produce the dimension the operation's rule defines rather than discard dimensional information.

## Units Within A Dimension

### A Dimension Groups Interconvertible Units

A dimension MUST admit more than one named unit, so that several units — such as a meter, a millimeter, and an inch — name measures of one dimension rather than each being a distinct dimension.

Two units of the same dimension MUST be interconvertible, and two units of different dimension MUST NOT be.

### A Unit Carries An Exact Scale To Its Dimension's Reference

Each unit of a dimension MUST carry an exact scale relating it to that dimension's reference unit, so that a conversion between two units of one dimension is an exact ratio rather than an approximation.

A conversion between two units of the same dimension MUST preserve the exact value when the underlying numeric type is exact, losing precision only where the underlying numeric type is itself inexact.

### A Named Unit's Conversion Is Unique

A named unit MUST resolve to a single dimension and a single scale, so that its conversion to and from its dimension's reference is a well-defined function rather than dependent on which of several declarations is consulted.

Declaring a named unit more than once with conflicting conversions — a differing dimension or a differing scale — MUST be a compile-time error, while a redeclaration that agrees is admissible.

### Combining Units Of One Dimension Is Well-Formed

Combining two quantities whose units share a dimension MUST be well-formed even when the units differ, the combination being taken at a common unit of that dimension reached by each operand's exact scale.

The result unit of a combination of same-dimension quantities MUST be a deterministic function of the operands' units, so that the result is reproducible rather than dependent on evaluation order.

### A Scaled Unit Is A Unit Scaled By An Exact Factor

A unit prefixed or otherwise scaled by an exact factor — a decimal multiple such as kilo or milli, or a binary multiple such as kibi or mebi — MUST itself be a unit of the same dimension as the unit it scales, differing only by that exact factor.

A scale factor MUST be an exact value, so that a prefixed unit converts to its base without approximation.

### A Stored Quantity Displays At Its Dimension's Reference Unit

A quantity stored at a scaled or named unit MUST, when it crosses the machine boundary as a value, display with its magnitude scaled to its dimension's reference unit and its unit shown as that reference, so that the number and the unit agree rather than disagreeing — a `5 kilometer` quantity displays as `5000 meter`, not the misleading `5 meter` that names the reference unit while keeping the source magnitude.

The display scale MUST be applied in the quantity's own inner numeric type, so that a Float rounds, a Rational stays exact, and an integer truncates toward zero on a non-whole ratio exactly as the numeric core's rules dictate — the dimensional layer introduces no arithmetic of its own beyond the source-denoted scale.

The reference-unit display MUST recurse into every quantity leaf of a compound value, so that a tuple, a sum payload, a nested compound, or a record field carrying a quantity each displays scaled to its reference independently and in its own inner type, not only a bare top-level quantity.

Scaling to the reference unit MUST be a display concern only and MUST NOT alter the stored magnitude, so that `Qty.value` returns the number the source wrote in the unit the source named, and an explicit `as`/`in` conversion computes by the exact direct ratio off that stored magnitude with no intermediate reference rounding.

## Layered, Not Core

### Dimensional Analysis Does Not Alter The Numeric Core

Attaching a unit to a numeric value MUST NOT change the value's numeric byte form.

Attaching a unit to a numeric value, or combining values that already share a unit, MUST NOT change the value's runtime behavior.

### A Unit Conversion Is The Arithmetic The Source Denotes

A conversion between two units of one dimension MUST be the scale arithmetic the source denotes by naming those units, not additional arithmetic the dimensional layer introduces, so that the emitted arithmetic is what the program means rather than an overhead the check imposes.

A unit conversion whose operands are compile-time constants MUST be computed at compile time, so that a conversion between constant quantities contributes no runtime arithmetic.

The dimension a quantity carries MUST be erased whether or not a scale conversion is emitted, so that the type-level dimensional information never survives into the component even when the scale arithmetic does.

### An Explicit Conversion Unwraps To A Bare Number

An explicit conversion of a quantity into a chosen unit — the `as`/`in` operation — MUST yield the dimensionless number counting how many of the chosen unit the quantity is, with the quantity wrapper removed, so that a conversion is the deliberate exit from the dimensional layer rather than a re-expression that stays dimensioned.

The result of an explicit conversion MUST be an ordinary number of the quantity's underlying numeric type, subject to ordinary numeric rules and no longer dimension-checked, so that once a program has asked "how many of this unit is it?" it holds the answer as a plain number and may combine it freely.

The chosen unit MUST share the quantity's dimension, so that a conversion across dimensions — a length into a duration — remains an error rather than silently producing a number.

## Optionality

### This Capability Is Optional

Dimensional analysis MUST be an optional capability a build may include or exclude, in accordance with the build's declared defaults.

### The Declared Default Is Include

When a build is not told whether to include dimensional analysis, it MUST include it.
