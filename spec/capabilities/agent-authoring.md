# Capability — Agent Authoring

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the affordances that make Cadenza easy for an agent to produce and transform: the
> structural interface, the round-tripping canonical form, and machine-readable compiler output. This
> is the realization of the language's top-priority purpose. Requirements realize
> [Core Principle X](../../constitution.md) and [Core Principle XI](../../constitution.md) and trace
> to [overview §13](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes what makes Cadenza a language an agent can write and read well: that the
canonical form round-trips byte-for-byte, that a structural interface lets an agent read and rewrite
a program's canonical representation without textual patching, that a structural edit either yields a
well-formed program or reports a machine-readable rejection, and that every output of the compiler is
machine-readable. It states these affordances as obligations, not the interface's concrete surface.

## Canonical Form

### The Canonical Form Round-Trips

Formatting a well-formed program MUST yield its byte-identical canonical textual form.

Parsing a program's canonical textual form and formatting the result MUST reproduce the same bytes.

## Structural Editing

### A Structural Interface Exists

The language MUST expose a documented interface to read and rewrite a program's canonical representation without textual patching.

A structural query or edit MUST operate without re-parsing code unrelated to its target.

### Structural Edits Preserve Well-Formedness Or Report

A structural edit MUST either yield a well-formed program or report a machine-readable rejection.

A structural edit MUST NOT yield a program that is malformed without reporting why.

## Machine-Readable Output

### Every Compiler Output Is Machine-Readable

The compiler MUST expose its diagnostics in a machine-readable form.

The compiler MUST expose the types it inferred in a machine-readable form.

The compiler MUST expose the capability manifest it produced in a machine-readable form.
