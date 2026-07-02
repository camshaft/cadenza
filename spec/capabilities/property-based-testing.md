# Capability — Property-Based Testing

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines property-based testing: generation, shrinking, reproducibility, and the use of
> contracts as oracles. Requirements realize [Core Principle III](../../constitution.md) and
> [Core Principle VIII](../../constitution.md) and trace to [overview §12](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes how a program states properties that should hold across many inputs and how the
tooling checks them: that generation is reproducible from a recorded seed, that a refinement
constrains the values generated for it, that a failing property shrinks to a minimal counterexample,
and that a stated postcondition can serve as a property oracle. It states the behavior of the test
harness, not its implementation.

## Generation

### Generation Is Seeded And Reproducible

A property run MUST be reproducible from its recorded seed, producing the same inputs on every
conforming run.

A reported property failure MUST record the seed and the input that produced it.

### Refinements Constrain Generation

A generator for a value of a refined type MUST produce only values satisfying that type's refinement.

## Shrinking

### Shrinking Converges To A Minimal Failing Input

When a property fails, the harness MUST search for a smaller input that still fails.

The shrinking search MUST terminate rather than search unboundedly.

The shrinking search MUST report a minimal failing input.

## Contracts As Oracles

### A Postcondition Is Usable As A Property

A declared postcondition MUST be usable as a property oracle without the author restating it as a separate assertion.

## Optionality

### Property-Based Testing Is Optional

Property-based testing MUST be an optional capability a build may include or exclude, in accordance with the build's declared defaults.

### The Declared Default Is Include

When a build is not told whether to include property-based testing, it MUST include it.
