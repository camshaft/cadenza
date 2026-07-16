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

## Coverage

### Exhaustive Coverage Is A Proof Over A Bounded Domain

A property whose inputs range over a bounded finite domain MAY be checked by enumerating that entire domain.

When a property is checked by enumerating its entire bounded finite domain, a run that finds no failing input MUST be treated as a proof of the property over the domain rather than as a sample.

### An Unbounded Domain Declines Exhaustive Checking

A property requested to be checked exhaustively over an unbounded input domain MUST be declined with a diagnostic rather than silently sampled, so that an exhaustive result is never reported for a domain that was not fully covered.

## Contracts As Oracles

### A Postcondition Is Usable As A Property

A declared postcondition MUST be usable as a property oracle without the author restating it as a separate assertion.

### Permutation Invariance Is A Property

A statement that a permutation of a fold's inputs produces a byte-identical result MUST be expressible as a property the generator exercises, so that order-independence can be checked by property testing as one rung in discharging it.

## Optionality

### Property-Based Testing Is Optional

Property-based testing MUST be an optional capability a build may include or exclude, in accordance with the build's declared defaults.

### The Declared Default Is Include

When a build is not told whether to include property-based testing, it MUST include it.
