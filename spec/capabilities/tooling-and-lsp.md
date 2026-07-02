# Capability — Tooling And Editor Integration

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the obligations of the language's editor and analysis tooling: that it shares one
> compiler, that incremental results equal batch results, and that queries over incomplete source are
> total. Requirements realize [Core Principle IX](../../constitution.md) and
> [Core Principle XI](../../constitution.md) and trace to [overview §10](../overview.md) and
> [overview §13](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes that the tooling an editor drives — hover, completion, go-to-definition,
diagnostics — is not a second implementation of the language but a view onto the one compiler and the
one executable semantics, that an incremental result equals what a full compilation would produce,
and that a query over source mid-edit returns a defined partial result rather than failing. It states
these obligations, not the transport or the editor protocol.

## Shared Semantics

### Tooling Shares The Compiler And The Semantics

A type, definition, or diagnostic the tooling reports MUST agree with the compiler and the executable semantics rather than be computed by a separate implementation.

## Incrementality

### Incremental Equals Batch

An incremental analysis result MUST equal the result a full compilation of the same source would produce.

An incremental analysis MUST NOT report a type, definition, or diagnostic that a full compilation would not.

## Robustness

### Queries Over Incomplete Source Are Total

A tooling query over source that does not fully parse MUST return a defined partial result rather than fail opaquely.

A tooling query MUST NOT crash the editor session on malformed source.
