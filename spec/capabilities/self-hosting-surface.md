# Capability — The Self-Hosting Surface

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the **self-hosting surface**: what the language must expose so that a Cadenza
> toolchain — the compiler the bootstrap targets — can be authored in Cadenza and walk a program as
> data (`spec/bootstrap.md`, `spec/capabilities/self-hosting-and-bootstrap.md`). Requirements realize
> [Core Principle IX](../../constitution.md), [Core Principle X](../../constitution.md), and
> [Core Principle XIV](../../constitution.md) and trace to [overview §10](../overview.md),
> [overview §11](../overview.md), and [overview §15](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading. The concrete primitive set, the reader/printer, and
> the compiled-component packaging are pinned at the declared-default location.

## Purpose And Scope

The behavioral oracle is the executable semantics as recorded by the conformance corpus, and the
independence of the judgment comes from two implementations of the compiler that must agree
(self-hosting-and-bootstrap.md). The bootstrap targets a Cadenza-authored **compiler** as its first
Cadenza artifact, so this capability fixes the surface a toolchain that walks a program as data needs:
the language exposes its own abstract syntax as ordinary values so a compiler authored in it can walk
a program; a run's observable behavior is a value with a canonical byte form so two implementations
agree exactly when they produce equal behavior; a program in canonical representation is reachable
from text by a reader and rendered by a printer that round-trip; and a program's input and output
cross the component boundary as bytes, not as a toolchain's internal values. It states these
invariants; the concrete primitive set, the reader/printer, and the compiled-component packaging are
declared defaults. The compiler is a component that itself reaches no host function — it derives, reads,
prints, and renders as exports of its own interface (build-tool-interface.md §The Compiler Exposes
Reader, Printer, And Display As Exports).

## Observable Behavior Is A Value

### Observable Behavior Is Represented As A Value

The observable behavior a program produces MUST be represented as a value whose canonical byte form is the observable behavior, so that two implementations agree exactly when they produce equal behavior for the same program.

The value representing observable behavior MUST distinguish the run's terminal condition from the ordered sequence of host calls the run made.

### A Compiled Program Computes Its Behavior Without Ambient Authority

A program's observable behavior MUST be a function of its canonical representation, its inputs, and the responses to the host calls it makes alone, so that the same program on the same inputs and the same responses produces the same behavior wherever it runs.

A program MUST NOT reach a host function outside the capabilities its manifest enumerates to compute its observable behavior, so that behavior is deterministic and capability-bound.

## The Abstract Syntax Is Reachable As A Value

### A Program's Syntax Tree Is An Ordinary Value

A program's abstract syntax tree MUST be expressible as an ordinary value of the language, so that a compiler authored in the language can examine a program without a foreign representation.

A compiler MUST be able to determine a node's kind and obtain its children from that value, so that it can walk the tree structurally.

### The Language Expresses A Compiler Over Its Own Syntax

The language MUST provide the constructs a compiler requires to be written in it, including declaring a value's kind, recursing over a tree, and comparing values.

The set of constructs the language must provide for the Cadenza-authored compiler to be authored in it MUST be pinned at the declared-default location, so that two builds agree on what the self-hosting surface may rely on.

## Text Is A Projection Reached By A Reader And A Printer

### A Reader Converts Text To The Canonical Representation

A reader MUST convert the text of a program to the program's canonical representation, so that a program can be written as text before a surface syntax exists.

A reader MUST NOT be required in the path that derives a component, consistent with the ast-encoding contract keeping parsing out of the compiler's trusted path.

### A Printer Renders The Canonical Representation As Re-Readable Text

A printer MUST render a program's canonical representation as text that a reader converts back to the same canonical representation.

Reading the text a printer produced for a value MUST yield a value equal to the original under structural equality, so that the reader and printer round-trip.

### The Result Crosses The Boundary As Its Proper Type

A compiled program's entry MUST export its result as the result's proper component type, so that the boundary is strictly, statically typed rather than a dynamically-tagged value.

The compiler MUST NOT collapse a typed result to an untyped string at the boundary in place of the result's proper component type, so that static typing is enforced at the boundary rather than deferred to a stringly-typed convention.

### Rendering A Result Is A Compiler-Exposed Display Conversion

A harness that displays a program's result MUST obtain that result's canonical text by invoking the compiler-exposed display conversion over the typed result, so that rendering knowledge stays in the compiler and the harness handles only the typed result and the returned text.

The display conversion the compiler exposes MUST render every value form the type system admits, so that the display of a value is the compiler's concern and grows with the type system rather than with the host.

### The Reader, Printer, And Display Are Compiler-Exposed Surfaces

The reader, printer, and display conversion MUST be surfaces the compiler exposes — text-to-canonical-binary, canonical-binary-to-text, and typed-result-to-text — rather than logic any host embeds, so that the knowledge of a value's textual form lives in the compiler and a host stays value-agnostic (host-interface-binding.md §The Host Formats Nothing).

The text form the printer produces for a value MUST be the value's canonical text form, so that two runs producing structurally-equal values print identical text.

## The Component Boundary Carries Bytes, Not Internal Values

### A Toolchain's Internal Values Do Not Cross The Boundary

A compiler's derivation interface MUST accept its input and produce its output as byte sequences at the component boundary, so that a toolchain's internal values do not cross it.

A compiled component MUST compute its observable behavior by running inside the component rather than by accepting an already-computed behavior across the boundary.

### Host Calls Reach The Host Through The Manifest's Capabilities

A compiled component MUST make the host calls its observable behavior records only through the host functions the program's manifest enumerates.

A compiled component MUST NOT make a host call through a host function the program's manifest does not enumerate.
