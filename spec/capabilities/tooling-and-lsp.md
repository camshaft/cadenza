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

## The Compiler Is A Queryable Oracle

### An Agent Queries The Compiler For Any Static Fact

An agent MUST be able to query the compiler for any static fact about a program — the type of any node, a name's resolution, the inferred manifest/effect row, the solved constraints — and the answer MUST be total, deterministic, and equal to what a full compile determines, so that an agent learns a static fact by asking rather than by instrumenting the program.

## Deterministic Replay Is The Debugger

### A Runtime Fact Is Observed By Replay

Because a run's observable behavior is a deterministic function of its input and its host-call responses, an agent MUST be able to observe any runtime fact of a run by replaying it from those recorded inputs rather than by inserting observation code.

### A Replayed Debug View Is Semantically Inert

A debug view reconstructed by replay MUST NOT be part of a program's observable behavior, so that debugging is a tool-time projection and does not change what the program means.
