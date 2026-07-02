# Frozen Contract — Determinism And Fuel

> **FROZEN CONTRACT.** This document pins the execution-level determinism the emitted code must
> guarantee and the resource accounting that bounds it, at the level of what the compiler emits
> rather than what the source says. It is the contract a component's byte-identical replay
> depends on. It is versioned and changed only by the coordinated act described in the
> constitution's Governance Floors. Its requirements realize [Core Principle III](../../constitution.md)
> and [Core Principle V](../../constitution.md) and trace to [overview §4](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract names execution properties, not a concrete engine; the concrete
> resource measure and numeric mode are pinned at the declared-default location.

## Purpose And Scope

Determinism and bounded termination are guarantees about emitted code, not just about source. A
program that declares no nondeterministic capability can still compile to a component whose output
varies if the compiler emits an instruction with an unspecified result, or whose termination depends
on timing if a loop is not accounted against a resource measure. This contract pins what the compiler
must and must not emit so that, given the same input and the same responses to a program's declared
capabilities, two runs produce identical bytes, and so that termination is bounded by a deterministic
measure. It concerns the nondeterminism the compiler must not introduce on its own; a source of
nondeterminism a program obtains through a declared capability is legible in its manifest and is the
running system's to permit. It does not pin the concrete measure or numeric mode, which are declared
defaults.

## Deterministic Emission

### No Nondeterministic Instruction Is Emitted

The compiler MUST NOT emit an instruction whose result is unspecified or implementation-defined across conforming runtimes.

The compiler MUST NOT emit an instruction that reads uninitialized memory.

The compiler MUST NOT emit a shared-memory or thread-spawning operation that the program did not obtain through a declared capability.

### Floating-Point Emission Is Determinism-Constrained

The compiler MUST emit floating-point operations under a single fixed rounding mode.

The compiler MUST NOT emit a fused or contracted floating-point operation whose result varies from the separately-rounded operations across conforming runtimes.

The compiler MUST emit floating-point operations such that a not-a-number result has a canonical bit pattern rather than a runtime-dependent one.

## Resource Accounting

### Every Unbounded Construct Consumes The Resource Measure

The compiler MUST emit code such that each loop iteration consumes the deterministic resource measure.

The compiler MUST emit code such that each function call consumes the deterministic resource measure.

The compiler MUST NOT emit a construct whose execution can proceed unboundedly without consuming the resource measure.

### Exhaustion Halts Deterministically

The compiler MUST emit code such that exhausting the resource measure halts execution at a defined point.

The point at which exhaustion halts execution MUST be a deterministic function of the input and the measure, not of wall-clock timing.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract MUST be additive with respect to already-derived components, or else carry an explicit version increment and a stated migration path.
