# Frozen Contract — Determinism And Fuel

> **FROZEN CONTRACT.** This document pins the execution-level determinism the emitted code must
> guarantee, at the level of what the compiler emits rather than what the source says. It is the
> contract a component's byte-identical replay depends on. It is versioned and changed only by the
> coordinated act described in the constitution's Governance Floors. Its requirements realize
> [Core Principle III](../../constitution.md) and trace to [overview §4](../overview.md).
>
> The filename retains the word "fuel" for citation stability; the resource-accounting requirements
> this contract once carried were retired by constitution Amendment 0.7.0, which delegates bounding a
> run's execution to the environment that hosts a component rather than to what the compiler emits, so
> only the deterministic-emission requirements remain here.
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract names execution properties, not a concrete engine; the numeric mode
> is pinned at the declared-default location.

## Purpose And Scope

Determinism is a guarantee about emitted code, not just about source. A program that declares no
nondeterministic capability can still compile to a component whose output varies if the compiler emits
an instruction with an unspecified result. This contract pins what the compiler must and must not emit
so that, given the same input and the same responses to a program's declared capabilities, two runs
produce identical bytes. It concerns the nondeterminism the compiler must not introduce on its own; a
source of nondeterminism a program obtains through a declared capability is legible in its manifest and
is the running system's to permit. It does not pin the numeric mode, which is a declared default.

Bounding a run against a resource measure is deliberately *not* pinned here: whether and how a run's
execution is bounded is a property of the environment that hosts a component, not of what the compiler
emits (constitution Core Principle V, retired by Amendment 0.7.0). The concrete engine's resource
metering is recorded at the declared-default location rather than required here.

## Deterministic Emission

### No Nondeterministic Instruction Is Emitted

The compiler MUST NOT emit an instruction whose result is unspecified or implementation-defined across conforming runtimes.

The compiler MUST NOT emit an instruction that reads uninitialized memory.

The compiler MUST NOT emit a shared-memory or thread-spawning operation that the program did not obtain through a declared capability.

### Floating-Point Emission Is Determinism-Constrained

The compiler MUST emit floating-point operations under a single fixed rounding mode.

The compiler MUST NOT emit a fused or contracted floating-point operation whose result varies from the separately-rounded operations across conforming runtimes.

The compiler MUST emit floating-point operations such that a not-a-number result has a canonical bit pattern rather than a runtime-dependent one.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-derived components, or else carry an explicit version increment.

A change to this contract that is not additive with respect to already-derived components MUST carry a stated migration path.
