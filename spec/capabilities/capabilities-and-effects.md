# Capability — Capabilities And Effects

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the mandatory declaration of a program's capabilities and the optional layer that
> tracks effects in the type system. Requirements realize [Core Principle IV](../../constitution.md)
> and [Core Principle VIII](../../constitution.md) and trace to [overview §6](../overview.md) and
> [overview §12](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes two things kept deliberately separate. The mandatory core: a program declares
every host capability it requires, and reaching an undeclared capability is a compile-time error —
this is how "no ambient authority" becomes a property of the program, and it feeds the
host-interface-binding contract that makes the emitted imports mirror the manifest. The optional
layer: a program may annotate functions with the effects they perform, and when it does, the compiler
checks those annotations — this is a verification layer, meaning-preserving and opt-in, not part of
the mandatory floor.

## Capability Declaration

### Capabilities Are Declared Up Front

A program MUST declare every host capability it requires in its capability manifest.

A capability a program requires but does not declare MUST be treated as not granted.

### Undeclared Capability Is A Compile-Time Error

A program that reaches a host operation its manifest does not enumerate MUST be rejected at compile time.

The compiler MUST determine a program's required capabilities from the operations it reaches, rather than from a separately-asserted list that could understate them.

### The Program Manifest Is The Union Of Its Modules

A program's capability manifest MUST be the union of the capabilities its constituent modules declare.

Dependency resolution MUST NOT introduce a capability that no module in the program declared.

## Effect Tracking

### Effect Tracking Is An Opt-In Layer

A program MUST compile without any effect annotation, using only the mandatory capability declaration.

A program MAY annotate a function with the effects it performs.

### Declared Effects Are Checked

When a function carries an effect annotation, the compiler MUST reject the program if the function performs an effect the annotation does not permit.

When a function carries an effect annotation, the compiler MUST reject the program if the function's callers do not account for the effects it declares.

### The Layer Preserves Meaning

Adding an effect annotation to a program that already compiles MUST NOT change the program's runtime meaning.

> **Open question — effect *checking* vs. effect *handling* (deferred).** The requirements above pin
> only the *checking* flavor of effects: an annotation is verified and, per "The Layer Preserves
> Meaning", is inert at runtime. They deliberately do **not** yet decide whether effect tracking also
> includes an *operational* flavor — establishing a context that is threaded implicitly through the
> call stack and accessed by a callee deep within it (as algebraic-effect handlers, an implicit reader
> context, or capability-passing provide), which *would* have runtime meaning. This is distinct from
> the mandatory capability declaration, which is about **host imports crossing the component boundary**;
> effects here are an **intra-program** concern. Because effect tracking is an optional capability the
> seed does not realize (`options/realized-capability-set/`), this distinction need not be resolved to
> bootstrap; it is an open point for the generation that first realizes effect tracking, and its
> resolution is a declared default recorded under `options/` at that time (per build-modes.md §"An Open
> Point Carries A Declared Default"). This note is descriptive; it adds no requirement.

## Optionality

### Effect Tracking Is Optional

Effect tracking MUST be an optional capability that a build may include or exclude, in accordance with the build's declared defaults.

### The Declared Default Is Include

When a build is not told whether to include effect tracking, it MUST include it.
