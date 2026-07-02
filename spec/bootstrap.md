# Bootstrap — Seed Toolchain And The Line Of Sight To Self-Hosting

> **BOOTSTRAP SPECIFICATION.** This document defines how Cadenza comes to exist and comes to build
> itself: the seed toolchain that has no Cadenza to compile it, the reference interpreter that
> becomes the behavioral oracle, the two derivation modes, and the bar an ignition must clear — a
> real, executed derivation, not the appearance of one. It fixes the human-versus-toolchain seam
> that resolves the bootstrap regress and the ignition subset the seed gate checks. Requirements
> realize [Core Principle XIV](../constitution.md), [Core Principle IX](../constitution.md), and
> [Core Principle XV](../constitution.md) and trace to [overview §11](./overview.md) and
> [overview §15](./overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. The concrete seed language and staging plan are pinned at the declared-default
> location.

## The Line Of Sight

The path from this specification to a Cadenza that builds itself is: a seed compiler in a foreign
language derives the first Cadenza toolchain to a component; the reference interpreter is authored in
Cadenza and derived by that seed, becoming the executable semantics' realization and the behavioral
oracle; agents extend the Cadenza source of the language and compiler, each generation derived by the
one before it and passed through both gates; and self-hosting is reached when the compiler is itself
authored in Cadenza. The concrete realization of each step is the declared bootstrap-strategy default.

## The Human-Versus-Toolchain Seam

### The First Toolchain Is Operator-Synthesized

The first Cadenza toolchain MAY be authored in a foreign language, because no Cadenza toolchain yet exists to derive it.

The first toolchain's output MUST satisfy the same determinism, capability-binding, bounded-termination, and reproducibility guarantees required of every later generation.

### The Toolchain Builds The Next Generation, Not Itself At Genesis

The seed toolchain MUST be able to derive a later generation of the toolchain from Cadenza source.

The seed toolchain MUST NOT be required to compile itself, so that the regress of "Cadenza needs Cadenza to build" is resolved by the operator-synthesized seed rather than by circular derivation.

## The Reference Interpreter As Oracle

### The Interpreter Realizes The One Semantics

The reference interpreter MUST implement exactly the behavior the executable-semantics corpus records.

The reference interpreter MUST be authored in Cadenza once the seed toolchain can derive it, so that the single semantics is a Cadenza artifact rather than foreign-language prose.

### Compiled Output Agrees With The Interpreter

An ignition MUST demonstrate that the compiled output of a program exhibits the same observable behavior as the reference interpreter over the same input, so that oracle agreement (Core Principle XIV) is exercised rather than assumed.

## Derivation Modes At Bootstrap

### Interpreted Derivation Is Available First

The toolchain MUST be able to derive a working component by embedding the reference interpreter over a program's canonical source before ahead-of-time compilation is complete.

A component derived by embedding the reference interpreter MUST satisfy every guarantee a compiled component satisfies.

### Compiled Derivation Is An Oracle-Checked Optimization

Ahead-of-time compilation MUST agree with the reference interpreter on every executable-semantics case before a generation using it is promoted.

## The Ignition Bar

### Ignition Demonstrates A Real End-To-End Derivation

An ignition MUST demonstrate that a Cadenza source program is derived to a content-addressed component and that the component is actually run to produce its output.

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
