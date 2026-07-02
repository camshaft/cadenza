# Capability — Build Modes And Ambiguity Resolution

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines how a build that takes the specification to a working compiler is driven: the two
> modes it runs in, how each mode treats a specification ambiguity, the declared defaults that make an
> unattended build deterministic, and the operator-gated points that must be resolved in the
> specification before an unattended build can run. Requirements realize
> [Core Principle XII](../../constitution.md) and [Core Principle XIV](../../constitution.md) and
> trace to [overview §15](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

The specifications are the durable artifact and the compiler is a rebuildable projection of them, so
the moment a build must choose something the specification did not decide is the moment that matters.
There are two kinds of driver: an author who is working on the specification and can resolve an
ambiguity at its source, and a user who only wants a working compiler and cannot. This spec fixes how
a build serves each: an attended build halts and hardens the specification, while an autonomous build
never stops to ask a question its driver cannot answer and instead proceeds on choices the
specification has already declared. It complements
[conformance-gate.md](./conformance-gate.md), which fixes when a build is promotable, and
[bootstrap.md](../bootstrap.md), whose operator-driven seed toolchain is the archetypal attended
build.

## Two Build Modes

### A Build Runs In One Of Two Modes

A build MUST run in exactly one of two modes: an attended mode driven by an author who can resolve ambiguity, or an autonomous mode driven by a user who cannot.

The selected build mode MUST be fixed for the duration of a build.

The selected build mode MUST be recorded in the build's decision record.

## Attended Mode Resolves Ambiguity At The Source

### Attended Mode Halts And Hardens The Specification

In attended mode, a build that reaches a specification ambiguity MUST halt rather than choose a resolution silently.

In attended mode, a build that halts on a specification ambiguity MUST surface that ambiguity to a human or another agent to resolve.

A resolution reached in attended mode MUST be folded into the specification as a new requirement before the build proceeds, so that the same ambiguity cannot recur.

After folding a resolution into the specification, an attended build MUST restart from the corrected specification rather than continuing from the point at which it halted.

## Autonomous Mode Never Halts On Ambiguity

### Autonomous Mode Applies A Declared Default Instead Of Asking

In autonomous mode, a build that reaches a specification ambiguity MUST NOT halt to ask a human to resolve it.

An autonomous build MUST resolve a specification ambiguity by applying the point's declared default.

An autonomous build that applies a declared default MUST record that it applied it.

An autonomous build that reaches an open point carrying no declared default MUST NOT halt to a human.

An autonomous build that reaches an open point carrying no declared default MUST record the missing default as a specification defect.

An autonomous build that reaches an open point carrying no declared default MUST proceed on a conforming choice.

## Open Points Carry Declared Defaults

### An Open Point Carries A Declared Default

A specification point that a conforming generation could resolve in more than one way MUST carry a declared default that states the conforming choice to apply when the point is otherwise unresolved.

A declared default MUST state either the conforming choice itself, or, where the standalone rule forbids naming an implementation choice in a specification, the location outside the specification at which that default is pinned.

### Declared Defaults Make An Autonomous Build Deterministic

Two autonomous builds of the same specification MUST apply the same declared defaults and therefore resolve every open point identically.

An autonomous build MUST record in its decision record every declared default and every user-facing default it applied, so that the assumptions behind the produced compiler are auditable.

## User-Facing Choices Are Not Ambiguities

### A User-Facing Choice May Be Asked In Either Mode

A choice the specification deliberately leaves to the deployer, such as the target language or the runtime engine, MUST be distinguished from a specification ambiguity.

A user-facing choice MAY be asked of the user in either mode.

A user-facing choice MUST carry a declared default so that a non-interactive or autonomous build can proceed without asking.

## Optional Capabilities Are Included Or Excluded Per Build

### A Capability May Declare Itself Optional

A capability specification MAY declare itself optional, meaning a conforming build MAY include or exclude the whole capability and still conform.

Whether to include an optional capability MUST be treated as a user-facing choice rather than a specification ambiguity.

Whether to include an optional capability MUST carry a declared default so that an autonomous or non-interactive build can proceed without asking.

### A Build Records Which Optional Capabilities It Included

A build MUST record in its decision record every optional capability it included and every optional capability it excluded, so that which capabilities the produced compiler contains is auditable.

An excluded optional capability MUST leave no trace in the produced compiler beyond the decision record's note that it was excluded, so that excluding a capability and never specifying it yield the same compiler.

## Operator-Gated Points Precede An Autonomous Build

### Some Points Only An Operator May Resolve

A specification point whose resolution the constitution reserves to an operator, including the core symbol namespace and a frozen-contract byte-level pin, MUST NOT be resolved by an autonomous build.

An autonomous build MUST verify, before it begins, that every operator-gated point its target depends on is already resolved in the committed specification.

An autonomous build that finds an unresolved operator-gated point MUST stop with a report that the specification is not yet ready for an autonomous build.

An autonomous build MUST NOT invent the resolution of an operator-gated point.

An autonomous build MUST NOT ask the user to supply an operator-gated point resolution.
