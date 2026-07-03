# Capability — Bootstrap Interpreter

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines what the reference interpreter is as an artifact and the **self-hosting surface**:
> what the language must expose so that a Cadenza toolchain — the compiler the bootstrap targets — can
> be authored in Cadenza and walk a program as data (`spec/bootstrap.md`,
> `spec/capabilities/self-hosting-and-bootstrap.md`). Requirements realize
> [Core Principle IX](../../constitution.md), [Core Principle X](../../constitution.md), and
> [Core Principle XIV](../../constitution.md) and trace to [overview §10](../overview.md),
> [overview §11](../overview.md), and [overview §15](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading. The concrete primitive set, the reader/printer, and
> the derived-component packaging are pinned at the declared-default location.

## Purpose And Scope

The single executable semantics is realized as a reference interpreter that is the behavioral oracle
(self-hosting-and-bootstrap.md). The bootstrap targets a Cadenza-authored **compiler** as its first
Cadenza artifact — not a Cadenza-authored interpreter — so this capability fixes the surface a toolchain
that walks a program as data needs, whether that toolchain interprets or compiles: the reference
interpreter is a pure function from a program's canonical representation to its observable behavior; the
language exposes its own abstract syntax as ordinary values so a toolchain authored in it can walk a
program; a program in canonical representation is reachable from text by a reader and rendered by a
printer that round-trip; and the observable behavior the interpreter computes crosses the component
boundary as bytes, not as interpreter-internal values. It states these invariants; the concrete
primitive set, the reader/printer, and how a derived component embeds the interpreter are declared
defaults.

## The Interpreter Is A Pure Function To Observable Behavior

### The Interpreter Maps A Program To Its Observable Behavior

The reference interpreter MUST be a function from a program's canonical representation and its inputs to the program's observable behavior.

The reference interpreter MUST compute a program's observable behavior without performing an effect outside the value it returns, so that evaluating a program and observing its behavior needs no host capability.

### Observable Behavior Is Represented As A Value

The observable behavior the interpreter produces MUST be represented as a value whose canonical byte form is the observable behavior, so that two interpreters agree exactly when they return equal values.

The value representing observable behavior MUST distinguish the run's terminal condition from the ordered sequence of events the run emitted.

## The Abstract Syntax Is Reachable As A Value

### A Program's Syntax Tree Is An Ordinary Value

A program's abstract syntax tree MUST be expressible as an ordinary value of the language, so that an interpreter authored in the language can examine a program without a foreign representation.

An interpreter MUST be able to determine a node's kind and obtain its children from that value, so that it can walk the tree structurally.

### The Language Expresses An Interpreter Over Its Own Syntax

The language MUST provide the constructs an interpreter requires to be written in it, including declaring a value's kind, recursing over a tree, and comparing values.

The set of constructs the language must provide for an interpreter to be authored in it MUST be pinned at the declared-default location, so that two builds agree on what the bootstrap interpreter may rely on.

## Text Is A Projection Reached By A Reader And A Printer

### A Reader Converts Text To The Canonical Representation

A reader MUST convert the text of a program to the program's canonical representation, so that a program can be written as text before a surface syntax exists.

A reader MUST NOT be required in the path that derives a component, consistent with the ast-encoding contract keeping parsing out of the compiler's trusted path.

### A Printer Renders The Canonical Representation As Re-Readable Text

A printer MUST render a program's canonical representation as text that a reader converts back to the same canonical representation.

Reading the text a printer produced for a value MUST yield a value equal to the original under structural equality, so that the reader and printer round-trip.

## The Component Boundary Carries Bytes, Not Interpreter Values

### The Interpreter's Values Do Not Cross The Boundary

A component that embeds the interpreter MUST accept its input and produce its output as byte sequences at the component boundary, so that the interpreter's internal values do not cross it.

A component that embeds the interpreter MUST compute observable behavior by interpreting inside the component rather than by accepting an already-computed behavior across the boundary.

### Emitted Events Reach The Host Through The Manifest's Capabilities

A component that embeds the interpreter MUST emit the events the interpreter's observable behavior records through the host capabilities the program's manifest enumerates.

A component that embeds the interpreter MUST NOT emit an event through a host capability the program's manifest does not enumerate.
