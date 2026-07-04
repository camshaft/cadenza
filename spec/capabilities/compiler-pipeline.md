# Capability — Compiler Pipeline

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the compiler's phases and the obligations each carries, and names the two gates a
> generation must pass. Requirements realize [Core Principle II](../../constitution.md),
> [Core Principle IX](../../constitution.md), and [Core Principle XII](../../constitution.md) and
> trace to [overview §7](../overview.md), [overview §10](../overview.md), and
> [overview §14](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes that the compiler proceeds through well-defined phases each of which is a
deterministic function of its input, that diagnostics from one phase do not abort the rest, and that
a generation is judged by two gates: the requirement gate (every load-bearing requirement cited by an
implementation and a test) and the behavior gate (every executable-semantics case reproduces its
output). It states the obligations phases carry; it does not prescribe the phase decomposition beyond
requiring that one exist and be respected.

## Representation

### The Compiler Operates On AST Values

The compiler MUST receive the program as an AST value obtained via quote or decode from the binary form.

The compiler MUST emit instructions as AST sum type values, not as string-tagged pseudo-structures.

The compiler MUST serialize instruction AST values to bytes through a recursive function operating on the AST sum type, so that instruction representation is deconstructible by pattern matching like any other Cadenza value.

## Phases

### The Pipeline Has Defined Phases

The compiler MUST proceed through phases each of which has a defined input and a defined output.

Each phase MUST produce output that is a deterministic function of its input.

### Phases Recover From Errors

A phase that encounters an error in one part of a program MUST record a diagnostic for that error.

A phase that encounters an error in one part of a program MUST continue processing the well-formed remainder rather than abort the whole compilation.

The compiler MUST report all diagnostics it can produce for a program rather than stop at the first.

## The Behavior Gate

### The Corpus Is A Gate

A build MUST fail if any executable-semantics case whose required capabilities the generation realizes does not reproduce its recorded output.

A behavior requirement MUST be discharged by executing the behavior and observing its output rather than by inspecting the shape of the code that implements it.

## The Requirement Gate

### Coverage Is A Gate

A generation in which any load-bearing requirement lacks both an implementation citation and a test citation MUST NOT be promoted.

The requirement gate MUST emit a machine-readable report from which the covered fraction of load-bearing requirements can be computed.

## The Two Gates Together

### Both Gates Must Pass

A generation MUST pass both the requirement gate and the behavior gate before it is promoted.

A generation that passes the requirement gate while failing the behavior gate MUST NOT be promoted.
