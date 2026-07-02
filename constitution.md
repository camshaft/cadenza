# Cadenza Constitution

> **What this document is.** The non-negotiable invariants of the Cadenza language and its
> compiler, stated as normative requirements. Every other specification — the frozen
> contracts and the capability specifications — inherits these and must not contradict them.
> The architecture these invariants serve is described in [spec/overview.md](./spec/overview.md);
> the vocabulary they use is defined in [spec/glossary.md](./spec/glossary.md).
>
> The key words **MUST**, **MUST NOT**, **SHALL**, **SHALL NOT**, **SHOULD**, **SHOULD NOT**,
> and **MAY** are to be interpreted as described in RFC 2119. Each requirement below is a
> single self-contained sentence under a stable heading, so it can be extracted and cited
> exactly. A requirement's identity is the tuple (this file, its section, its quoted
> sentence); there are no separate identifiers, and changing a sentence's wording flags every
> citation that no longer matches it.

## Core Principles

### I. Source Is The Authority, The Component Is Derived

A program's source MUST be treated as the sole authority over the program's meaning.

A compiled component MUST be treated as a rebuildable, content-addressed artifact rather than as the authority over its source.

A defect MUST be corrected by reworking the source or the specification and recompiling, rather than by editing an emitted component.

### II. Compilation Is Reproducible

Compiling the same canonical source with the same pinned toolchain MUST produce a byte-identical component.

The compiler MUST NOT embed a wall-clock time, an absolute filesystem path, or a build-host identifier into its output.

The compiler MUST emit its output in an order that is a function of the source alone, independent of filesystem enumeration order or nondeterministic collection iteration.

### III. The Compiler Introduces No Undeclared Nondeterminism

A compiled component MUST produce byte-identical output given the same input and the same responses to its declared capabilities, on every conforming runtime.

The compiler MUST NOT introduce into a component a source of nondeterminism that the program did not obtain through a declared capability.

The compiler MUST NOT emit an operation whose result depends on uninitialized memory.

The compiler MUST NOT emit an operation whose result depends on thread scheduling.

The compiler MUST emit each numeric operation with a fully specified result so that the operation does not vary between conforming runtimes.

### IV. No Ambient Authority

A compiled component MUST import only the host operations enumerated in its capability manifest.

The compiler MUST NOT emit an import that the program's declared capabilities do not enumerate.

A program that reaches a host operation it does not declare MUST be rejected at compile time rather than compiled to a component carrying a latent import.

### V. Bounded Termination By A Deterministic Measure

The compiler MUST emit code whose execution is accountable against a deterministic resource measure rather than against wall-clock time.

The compiler MUST emit code such that exhausting that resource measure halts execution at a defined point.

The compiler MUST NOT emit a construct whose consumption of the resource measure is unaccountable.

### VI. The Runnable Form Is A Verified, Content-Addressed Component

The runnable form of a program MUST be a content-addressed binary component behind a versioned host interface.

The compiler MUST bind each emitted component to a hash over the component's own bytes so that a third party can re-derive and verify it.

The compiler MUST emit a component that names the exact host-interface version it targets.

### VII. Strong Static Typing Is Mandatory

Every expression in a well-formed program MUST have a statically determined type before the program is compiled to a component.

The compiler MUST reject a program that is not well-typed rather than emit a component carrying a deferred type error.

The compiler MUST erase types from the emitted component so that the runnable form carries no runtime type reflection.

### VIII. Verification Is Progressive And Meaning-Preserving

A program MUST be compilable when only the core guarantees — static typing, determinism, and capability-safety — are satisfied.

An added verification layer MUST NOT change the runtime meaning of a program that already compiles without it.

The compiler MUST reject a program that states a verification obligation the compiler cannot discharge, rather than silently ignore that obligation.

### IX. Behavior Has One Executable Semantics

The behavior of every language construct MUST be defined by exactly one executable semantics that is its single source of truth.

A conformance test for a construct MUST be derivable from that executable semantics.

The compiler and every tool MUST agree with the executable semantics rather than encode a behavior of their own.

### X. Programs Are Readable By Agents And Humans

The canonical form of a program MUST be a stable binary serialization of its abstract syntax tree, such that a program has one canonical byte form independent of any textual rendering.

A textual syntax MUST be a lossless projection of the canonical form, such that parsing its text yields the canonical form and printing the canonical form yields text that parses back to the same canonical form.

The structure of a program MUST be manipulable through a documented structural interface without re-parsing unrelated code.

### XI. Diagnostics Are Machine-Actionable

Every diagnostic the compiler emits MUST carry a stable machine-readable code.

Every diagnostic the compiler emits MUST carry a precise source span.

Every diagnostic the compiler emits MUST name the rule or requirement it enforces so that an agent can act on it programmatically.

### XII. Specifications Are The Durable Artifact

Every normative statement about the language MUST be written as an RFC-2119 requirement extractable by the conformance gate.

The compiler MUST be treated as a regenerable projection of the specification rather than as the source of truth.

A regeneration in which any load-bearing requirement lacks both an implementation citation and a test citation MUST NOT be promoted.

A requirement that describes runtime behavior MUST be discharged by executing that behavior and observing its result, rather than by citing code whose shape resembles it.

### XIII. Normative Statements Are Atomic And Standalone

Each normative statement MUST carry exactly one obligation under a stable heading.

A normative statement MUST NOT name a concrete engine, a hashing algorithm, a numeric width, a prior prototype, or a source-file path.

A concrete technology choice MUST be recorded at the declared-defaults location rather than in a normative requirement.

### XIV. The Language Has A Line Of Sight To Self-Hosting

The executable semantics MUST be realizable as a reference interpreter that serves as the behavioral oracle for the language.

A compiled program's observable behavior MUST agree with the reference interpreter over the same input.

The toolchain MUST admit a staged path from a foreign-language seed compiler to a Cadenza-authored compiler in which each generation is derivable by the generation before it.

### XV. A Requirement Is Enforceable Or It Is Not A Requirement

A normative requirement MUST bind to a mechanism that detects its violation on a specific line.

A statement that no mechanism can enforce by binding to a specific line MUST NOT be written as a normative requirement.

A requirement that pins the shape of an artifact without an accompanying requirement that some path exercises that artifact MUST NOT be treated as sufficient, so that a modeled stand-in is a non-conforming implementation rather than a passing one.

## Governance Floors

These are the minimum-process floors that no evolution policy may lower. They exist because the discipline that governs how these specifications change is itself amendable; these floors bound that self-amendment.

### The Component ABI Changes Only By Coordinated Act

A change to the component ABI that alters the bytes produced from unchanged source MUST carry a version increment and a stated migration path.

A change to the component ABI MUST be evaluated against already-derived components before it ships.

### Determinism And Capability-Safety Are Never Downgradable

The determinism guarantee MUST NOT be reducible to a warning by any compiler configuration.

The capability-safety guarantee MUST NOT be reducible to a warning by any compiler configuration.

### Reproducibility Outranks Optimization

An optimization that would break byte-identical reproduction of a component MUST NOT be enabled in a conforming build.

### Amendment Discipline

An amendment to this constitution MUST be recorded with its rationale in the learnings record.

An amendment that weakens a governance floor MUST require explicit human approval.

## Governance

This constitution supersedes all other specifications where they conflict on an invariant. The frozen contracts under `spec/contracts/` pin the byte- and ABI-level forms these invariants govern; the capability specifications under `spec/capabilities/` describe behavior that must satisfy these invariants; the executable semantics under `spec/semantics/` is the single source of truth for behavior. Compliance is checked by two gates: the requirement gate, under which every load-bearing requirement here must carry an implementation citation and a test citation in any promoted generation, and the behavior gate, under which every executable-semantics case must reproduce its recorded output. Amendments follow the Amendment Discipline above and are traced against the architecture in [spec/traceability.md](./spec/traceability.md).

**Version**: 0.1.0 | **Ratified**: 2026-07-02 | **Last Amended**: 2026-07-02
