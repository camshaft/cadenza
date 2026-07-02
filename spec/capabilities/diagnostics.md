# Capability — Diagnostics

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the machine-readable diagnostics the compiler emits. Requirements realize
> [Core Principle XI](../../constitution.md) and [Core Principle II](../../constitution.md) and
> trace to [overview §13](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading. The concrete diagnostic record is pinned at the
> declared-default location.

## Purpose And Scope

This capability fixes that every diagnostic the compiler emits is machine-actionable: it carries a
stable code an agent can branch on, a precise span, and a reference to the rule it enforces, and the
diagnostics of a run are emitted in a deterministic order. It states these properties; the concrete
record shape is the declared diagnostics-schema default.

## Diagnostic Content

### Every Diagnostic Has A Stable Code

Every diagnostic the compiler emits MUST carry a machine-readable code that is stable across changes to unrelated diagnostics.

The code a diagnostic carries MUST NOT change when the diagnostic's human-readable wording changes.

### The Code Set Is Pinned Outside The Specification

The set of diagnostic codes and the rejection each code names MUST be pinned at the declared-default location so that two builds emit the same code for the same rejection.

A diagnostic code that an executable-semantics case references MUST resolve to an entry in that pinned code set.

### Every Diagnostic Has A Precise Span

Every diagnostic the compiler emits MUST carry a source span identifying the construct it concerns.

### Every Diagnostic Attributes A Rule

Every diagnostic the compiler emits MUST name the rule or requirement it enforces so that an agent can trace the diagnostic to its cause.

## Determinism

### Diagnostics Are Emitted In A Deterministic Order

The sequence of diagnostics the compiler emits for a program MUST be a deterministic function of the program's source.

## Machine-Readability

### Diagnostics Are Machine-Readable

The compiler MUST expose its diagnostics in a machine-readable form rather than only as human-formatted text.
