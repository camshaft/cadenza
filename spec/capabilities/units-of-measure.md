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

## Layered, Not Core

### Dimensional Analysis Does Not Alter The Numeric Core

Adding a unit to a numeric value MUST NOT change the value's numeric byte form.

Adding a unit to a numeric value MUST NOT change the value's runtime behavior.

## Optionality

### This Capability Is Optional

Dimensional analysis MUST be an optional capability a build may include or exclude, in accordance with the build's declared defaults.

### The Declared Default Is Include

When a build is not told whether to include dimensional analysis, it MUST include it.
