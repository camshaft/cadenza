# Capability — Conformance Gate

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the two gates that decide whether a regenerated compiler is real: the requirement
> gate that maps every normative sentence to the code and test that satisfy it, and the behavior gate
> that runs every executable-semantics case. Requirements realize
> [Core Principle IX](../../constitution.md), [Core Principle XII](../../constitution.md), and
> [Core Principle XV](../../constitution.md) and trace to [overview §14](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes how a generation is judged. The requirement gate extracts every normative
requirement by its quoted-sentence identity and requires each load-bearing one to be discharged by a
citation that actually performs and tests its behavior. The behavior gate runs every
executable-semantics case. Together they guard against the failure this whole design exists to
prevent: a compiler that reproduces the *shape* of the language — the right signatures, the right
citations — while its behavior is a stub that never runs. It states the gate's behavior, not its
implementation.

## Requirement Identity

### Identity Is The Quoted Sentence

A requirement's identity MUST be the tuple of its specification file, its section, and its exact quoted sentence, with no separate identifier.

A citation MUST record the requirement it satisfies by quoting the requirement's sentence, so that changing the sentence's wording invalidates every citation that no longer matches it.

### Requirements Are Written To Be Extractable

Every normative requirement MUST be written as a single self-contained RFC-2119 sentence under a stable section heading, so that it can be extracted unambiguously.

The gate MUST treat a citation whose quoted text matches no requirement as a failure that names the offending location.

## Enforceability

### Every Requirement Binds To An Enforcing Line

Every normative requirement MUST bind to at least one enforcing line that detects its violation, realizing Core Principle XV.

A statement that no mechanism can bind to an enforcing line MUST be written as descriptive prose rather than as a normative requirement.

The enforcing line for a requirement MUST be one of an implementation-and-test citation in a generation's source, a citation in the code that performs a governance act the requirement mandates, or a citation in the change-process check that guards a future edit the requirement constrains.

A requirement the gate holds not load-bearing for a generation MUST still bind to an enforcing line of another kind, so that a carve-out routes a requirement to a different enforcing mechanism rather than exempting it from enforcement.

## Requirements That Do Not Gate A Generation

### A Change-Process Requirement Is Not Load-Bearing For A Generation

A requirement that governs how the specification itself may change MUST NOT be counted as load-bearing for a generation's requirement gate.

A requirement that mandates a human governance act rather than a runtime behavior MUST NOT be counted as load-bearing for a generation's requirement gate.

A requirement excluded from a generation's load-bearing set by this section MUST be recorded in the pinned non-load-bearing set at the declared-default location, so that two builds judge against an identical load-bearing set.

## Coverage

### Coverage Requires Implementation And Test

A requirement MUST be counted as covered only when it has both an implementation citation and a test citation.

A cited test MUST exercise the behavior its requirement describes rather than merely reference the requirement's text.

### A Citation Discharges Its Own Requirement

An implementation citation MUST annotate the code that performs the cited requirement's behavior, rather than a placeholder whose only effect is to invoke code cited elsewhere.

A cited test MUST fail whenever the specific behavior the cited requirement describes is removed or violated, so that a test insensitive to its own requirement does not discharge it.

A cited test MUST distinguish a correct implementation of its requirement from one that violates it, rather than pass for reasons unrelated to that requirement.

Two requirements that describe distinct behaviors MUST NOT be discharged by citations that resolve to one shared check which cannot fail for one behavior without failing for the other.

A generator that emits citations MUST produce, for each requirement, an implementation site and a test that satisfy this section, rather than point many requirements at one undifferentiated exercise.

### A Behavior Requirement Is Covered Only By Execution

A requirement that describes runtime behavior MUST be discharged by a test that executes that behavior and observes its result.

A requirement that pins the shape of an artifact MUST NOT be counted as covered by a citation that produces the shape without a path that exercises it.

## The Behavior Gate

### Every Case Executes To Its Recorded Output

The behavior gate MUST execute every case in the executable-semantics corpus.

The behavior gate MUST fail a generation for which any case does not reproduce its recorded output.

### Every Behavioral Requirement Is Witnessed By A Case

Every behavioral requirement of an included capability MUST be witnessed by at least one case in the executable-semantics corpus.

A generation whose corpus omits a witnessing case for a load-bearing behavioral requirement MUST NOT be promoted.

### A Cited Behavioral Test Is Sensitive To Its Requirement

The gate MUST reject a generation in which a cited test of a behavioral requirement still passes when that requirement's behavior is broken.

## Promotion

### The Gate Is The Promotion Bar

A generation in which any requirement at the MUST or SHALL level is not covered MUST NOT be promoted.

A requirement at the SHOULD or MAY level MAY be left uncovered.

A generation MUST pass both the requirement gate and the behavior gate before it is promoted.

### The Requirement And Code Sides Are Separated

The requirement side of the gate configuration MUST list the specification files and be owned by hand rather than regenerated.

The code side of the gate configuration MUST be regenerated per target language.

A regeneration of the code side MUST NOT alter the requirement side.

### An Excluded Optional Capability Is Not Load-Bearing

A requirement belonging to an optional capability a build excluded MUST NOT be counted as load-bearing for that build's promotion.

A requirement belonging to an optional capability a build included MUST be load-bearing for that build exactly as a non-optional requirement of the same level is.

### The Optional-Capability Set Is Recorded And Defaults To Included

A gate run MUST record which optional capabilities the build included, so that the load-bearing set it judged against is reproducible.

A build that is not told whether to include an optional capability MUST be judged as having included it.

## Judging Against An Immutable Specification

### A Gate Run Judges Against A Content-Addressed Snapshot

A gate run MUST judge a generation against a content-addressed snapshot of the specification tree.

A gate run MUST record the snapshot it judged against, so that its verdict can be reproduced against the same specification.

## Advisory

### The Gate Should Detect An Insensitive Test Beyond The Behavioral Subset

The gate SHOULD detect a cited test that still passes when the requirement it discharges is broken, beyond the behavioral subset the gate already rejects, so that a citation insensitive to its own requirement is reported rather than counted.
