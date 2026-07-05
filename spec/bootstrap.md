# Bootstrap — Seed Toolchain And The Line Of Sight To Self-Hosting

> **BOOTSTRAP SPECIFICATION.** This document defines how Cadenza comes to exist and comes to build
> itself: the seed toolchain that has no Cadenza to compile it, the conformance corpus that is the
> behavioral oracle, the two compilers whose agreement supplies the judgment's independence, and the
> bar an ignition must clear — a real, executed derivation, not the appearance of one. It fixes the
> human-versus-toolchain seam
> that resolves the bootstrap regress and the ignition subset the seed gate checks. Requirements
> realize [Core Principle XIV](../constitution.md), [Core Principle IX](../constitution.md), and
> [Core Principle XV](../constitution.md) and trace to [overview §11](./overview.md) and
> [overview §15](./overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. The concrete seed language and staging plan are pinned at the declared-default
> location.

## The Line Of Sight

The path from this specification to a Cadenza that builds itself is direct: a reference compiler in a
foreign language lowers Cadenza source to a component and runs it, and the behavioral
oracle is the executable semantics as recorded by the conformance corpus; the first Cadenza source
that reference compiler compiles is the Cadenza-authored compiler; agents extend the Cadenza source of
the language and its compiler, each generation derived by the one before it and passed through both
gates; and self-hosting is reached when the Cadenza compiler is itself compiled by the previous
Cadenza compiler, with the foreign-language seed compiler no longer on the critical path. The seed
compiler is the one foreign-language artifact; the compiler authored in Cadenza is the first Cadenza
artifact. The two compilers — the foreign-language seed and the Cadenza-authored one — are the two
independent implementations whose agreement supplies the independence of the judgment (Core Principle
XIV), in place of an interpreter-versus-compiler differential. The concrete realization of each step is
the declared bootstrap-strategy default.

## The Human-Versus-Toolchain Seam

### The First Toolchain Is Operator-Synthesized

The first Cadenza toolchain MAY be authored in a foreign language, because no Cadenza toolchain yet exists to derive it.

The first toolchain's output MUST satisfy the same determinism, capability-binding, bounded-termination, and reproducibility guarantees required of every later generation.

### The Toolchain Builds The Next Generation, Not Itself At Genesis

The seed toolchain MUST be able to derive a later generation of the toolchain from Cadenza source.

The seed toolchain MUST NOT be required to compile itself, so that the regress of "Cadenza needs Cadenza to build" is resolved by the operator-synthesized seed rather than by circular derivation.

## The Oracle Is The Recorded Semantics

### The Conformance Corpus Is The Behavioral Oracle

The behavioral oracle MUST be the executable semantics as recorded by the conformance corpus, so that the authority a compiled program is judged against is the reviewed record of behavior rather than any one program that computes it.

The seed compiler and the Cadenza-authored compiler MUST each agree with the recorded semantics on every case the generation realizes, so that the definition of behavior is not any single implementation's own output.

### Two Compilers Supply The Independence Of The Judgment

The independence Core Principle XIV requires MUST be supplied by two implementations of the compiler — the foreign-language seed compiler and the Cadenza-authored compiler — whose compiled output MUST agree on the observable behavior of every program the generation realizes.

A reference interpreter MAY additionally serve as an independent oracle, but it MUST NOT be required for the semantics to be defined or for a compiled program to be judged.

### Compiled Output Agrees With The Recorded Semantics

An ignition MUST demonstrate that the compiled output of a program exhibits the observable behavior the executable semantics records for that program, so that oracle agreement (Core Principle XIV) is exercised rather than assumed.

## Derivation At Bootstrap

### The Seed Reference Compiler Is Native And Compiles Cadenza To A Component

The seed reference compiler MAY be a native program of the foreign seed language rather than a component, because its role is to lower Cadenza source to a component and run it, not to be a derived artifact itself.

The seed reference compiler MUST lower a program's canonical representation to a complete runnable component and run it on the host, so that a runnable component exists from the first foreign-language artifact without an interpreter being embedded in it.

A component the seed compiler produces MUST exhibit, on every executable-semantics case the generation realizes, the observable behavior the executable semantics records, before that generation is promoted.

### The Runtime Is One Sandboxed Component Runtime

Every program the toolchain runs MUST run as a component on the host, so that the language has one runtime and no separately-maintained execution engine defines behavior alongside it.

The seed MUST NOT define a program's behavior by directly evaluating its canonical representation in the foreign seed language, so that behavior is observed by running the compiled component rather than by a foreign-language tree-walk that would be a second definition of the language.

### The Self-Hosted Compiler Is Authored In Cadenza

The bytes of the component a self-hosting derivation produces MUST be produced by the Cadenza-authored compiler over the program's canonical representation, so that self-hosting rests on Cadenza compiling Cadenza rather than on the foreign-language seed.

The Cadenza-authored compiler MUST be able to construct component bytes as an ordinary value of a byte-sequence value form the language realizes, whose realized set is pinned at the declared-default location.

The bytes the Cadenza-authored compiler produces MUST be the complete runnable component rather than a partial artifact that a separate tool completes into the component, so that a derivation's byte output is a function of the Cadenza-authored compiler alone.

The compiler's derivation interface MUST take the program's binary AST as a byte sequence and yield either the component's bytes on success or a sequence of machine-readable diagnostics on rejection, so that a rejection returns actionable diagnostics rather than an opaque empty result and the interface is statically typed independent of how any compiler realizing it is itself hosted.

The rejection arm of the derivation interface MUST carry the diagnostics' machine-readable codes (Core Principle XI), so that a caller distinguishes a well-typed program that produced no component from an ill-typed program that was refused, and learns why.

Every stage that lowers a program toward component bytes, including any assembly of a textual or structured instruction form into its binary encoding, MUST be expressible in Cadenza, so that a self-hosting generation performs the whole translation and no part of it escapes to a foreign-language or external tool.

The Cadenza source the bootstrap authors MUST be written as a well-typed static program that satisfies the static-typing obligations of Core Principle VII, so that the seed compiler that enforces those obligations accepts it.

The Cadenza source the bootstrap authors MUST NOT rely on a dynamic idiom that a static type discipline would reject, such as runtime kind-reflection over a value in place of a sum type and a match, so that the source rests on the static type discipline the seed compiler enforces rather than on an idiom it would refuse.

### A Reference Interpreter Is An Optional Independent Oracle

The toolchain MAY additionally realize a reference interpreter that evaluates a program's canonical representation directly, as an independent oracle whose observable behavior can be cross-checked against a compiled component.

A reference interpreter, if realized, MUST agree with the executable semantics on every case it realizes, so that offering it adds an independent check rather than a second definition of behavior.

A reference interpreter MUST NOT be relied on as the runtime for any promoted generation, so that the language's one runtime remains the sandboxed component runtime.

## The Ignition Bar

### Ignition Demonstrates A Real End-To-End Derivation

An ignition MUST demonstrate that a Cadenza source program is derived to a content-addressed component.

An ignition MUST demonstrate that the derived component is actually run to produce its output.

An ignition MUST demonstrate that the derived component's imports mirror its declared capability manifest, so that the capability-binding is exercised rather than merely configured.

An ignition MUST demonstrate that re-deriving the same source with the same toolchain produces a byte-identical component, so that reproducibility is exercised rather than asserted.

### A Modeled Derivation Is Not An Ignition

An ignition demonstrated only by emitting the events that would accompany a derivation, without a component that was actually derived and run, MUST NOT be treated as a conforming ignition.

A subsystem whose specification can be satisfied without executing it MUST be exercised by the ignition's end-to-end path rather than stood in for by a placeholder.

## Turning The Flywheel

### A Generation Is Synthesized, Derived, Gated, And Run

A new generation MUST be produced by reading this specification, synthesizing Cadenza source, deriving it with the previous generation, and passing both gates, rather than by hand-editing an emitted component.

The claim that the system has evolved MUST be backed by a component that was actually derived, rather than by the appearance of a regeneration.

The claim that the system has evolved MUST be backed by a derived component that is now running.

### The Whole Regeneration Is Auditable

Each step by which a generation is produced MUST be recorded so that the regeneration is reconstructable and independently verifiable.
