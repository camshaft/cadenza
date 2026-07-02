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
well-formed program or reports a machine-readable rejection, that documentation is a first-class part
of the representation, and that every output of the compiler is machine-readable. It states these
affordances as obligations, not the interface's concrete surface.

## Canonical Form

### The Canonical Form Is The Binary AST

A program's canonical form MUST be the binary AST fixed by the ast-encoding contract, so that its identity is independent of any textual rendering.

An agent MUST be able to read and construct a program's canonical binary AST directly, without going through a textual syntax.

### Textual Syntaxes Round-Trip Through The Canonical Form

Parsing a textual rendering of a program MUST yield its canonical binary AST.

Printing a program's canonical binary AST in a textual syntax MUST yield text that parses back to the same canonical binary AST.

## Structural Editing

### A Structural Interface Exists

The language MUST expose a documented interface to read and rewrite a program's canonical representation without textual patching.

A structural query or edit MUST operate without re-parsing code unrelated to its target.

### Structural Addressing Is Deterministic

The address by which the structural interface identifies a node MUST be a deterministic function of the canonical representation.

A structural query MUST return a result that is a deterministic function of the canonical representation, so that an agent can target and re-target edits reproducibly.

### Structural Edits Preserve Well-Formedness Or Report

A structural edit MUST either yield a well-formed program or report a machine-readable rejection.

A structural edit MUST NOT yield a program that is malformed without reporting why.

## Documentation

### Documentation Is Part Of The Representation

Documentation attached to a definition MUST be carried in the canonical representation rather than discarded as lexical trivia.

Any definition MUST be able to carry documentation, so that every part of a program can be documented.

### Documentation Survives Round-Trip And Edits

Documentation MUST be preserved when a program's binary AST is printed to a textual syntax and parsed back.

A structural edit MUST preserve the documentation attached to a part of the program it does not change.

### Documentation Is Machine-Readable

The compiler MUST expose the documentation attached to a definition in a machine-readable form.

Documentation MUST NOT change the runtime meaning of a program.

## Machine-Readable Output

### Every Compiler Output Is Machine-Readable

The compiler MUST expose its diagnostics in a machine-readable form.

The compiler MUST expose the types it inferred in a machine-readable form.

The compiler MUST expose the capability manifest it produced in a machine-readable form.
