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

The seed compiler generation MUST realize the static-typing obligations of this section rather than defer them, because under the two-compiler bootstrap it compiles programs to components rather than evaluating them dynamically, so the dynamic-evaluation basis on which a seed generation formerly deferred typing no longer holds.

A program that is not well-typed MUST be rejected with the machine-readable diagnostic code for the type rule it violates, so that a type rejection is a compile-time event every generation makes rather than a runtime outcome only a dynamic evaluator would exhibit.

The static-typing obligations of this section MAY be realized incrementally over the type rules a generation's compiler covers, provided the compiler rejects rather than miscompiles a program that uses a rule it does not yet check, so that the type system grows without ever emitting a component carrying a deferred type error.

### VIII. Verification Is Progressive And Meaning-Preserving

A program MUST be compilable when only the core guarantees — static typing, determinism, and capability-safety — are satisfied.

An added verification layer MUST NOT change the runtime meaning of a program that already compiles without it.

The compiler MUST reject a program that states a verification obligation the compiler cannot discharge, rather than silently ignore that obligation.

### IX. Behavior Has One Executable Semantics

The behavior of every language construct MUST be defined by exactly one executable semantics that is its single source of truth.

A conformance test for a construct MUST be derivable from that executable semantics.

The compiler MUST agree with the executable semantics rather than encode a behavior of its own.

Every tool MUST agree with the executable semantics rather than encode a behavior of its own.

### X. Programs Are Readable By Agents And Humans

The canonical form of a program MUST be a stable binary serialization of its abstract syntax tree, such that a program has one canonical byte form independent of any textual rendering.

A textual syntax MUST be a lossless projection of the canonical form, such that parsing its text yields the canonical form and printing the canonical form yields text that parses back to the same canonical form.

The structure of a program MUST be manipulable through a documented structural interface without re-parsing unrelated code.

### XI. Diagnostics Are Machine-Actionable

Every diagnostic the compiler emits MUST carry a stable machine-readable code.

Every diagnostic the compiler emits MUST carry a precise source span.

Every diagnostic the compiler emits MUST name the rule or requirement it enforces so that an agent can act on it programmatically.

Every diagnostic that reports a rejection MUST carry a machine-applicable route to a compliant program, expressed as a structural edit of the program's abstract syntax tree rather than a textual patch.

A route whose application the compiler has confirmed recompiles the program clean and clears the diagnostic MUST be marked verified, and a route the compiler cannot so confirm MUST carry an applicability marker declaring it a heuristic, so that an agent can distinguish a guaranteed repair from a suggested one.

The route a diagnostic carries and its verified-or-heuristic status MUST be a deterministic function of the source, like every other compiler output.

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

The behavioral oracle for the language MUST be the executable semantics as recorded by its conformance corpus, against which a compiled program's observable behavior is judged.

A compiled program's observable behavior MUST agree with the executable semantics over the same input.

The independence of that judgment MUST be supplied by more than one implementation of the compiler that MUST agree with each other on the observable behavior of every program the generation realizes, so that no single implementation is both the definition of behavior and its own judge.

The executable semantics MAY additionally be realized as a reference interpreter serving as an independent oracle, but a reference interpreter MUST NOT be required for the semantics to be defined or for a compiled program to be judged.

The toolchain MUST admit a staged path from a foreign-language seed compiler to a Cadenza-authored compiler in which each generation is derivable by the generation before it.

### XV. A Requirement Is Enforceable Or It Is Not A Requirement

A normative requirement MUST bind to a mechanism that detects its violation on a specific line.

A statement that no mechanism can enforce by binding to a specific line MUST NOT be written as a normative requirement.

A requirement that pins the shape of an artifact without an accompanying requirement that some path exercises that artifact MUST NOT be treated as sufficient, so that a modeled stand-in is a non-conforming implementation rather than a passing one.

## Governance Floors

These are the minimum-process floors that no evolution policy may lower. They exist because the discipline that governs how these specifications change is itself amendable; these floors bound that self-amendment.

### The Component ABI Changes Only By Coordinated Act

A change to the component ABI that alters the bytes produced from unchanged source MUST carry a version increment.

A change to the component ABI that alters the bytes produced from unchanged source MUST carry a stated migration path.

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

**Version**: 0.6.0 | **Ratified**: 2026-07-02 | **Last Amended**: 2026-07-05

> **Amendment 0.2.0 (2026-07-02).** Core Principle VII gains a bootstrap carve-out: the
> operator-synthesized seed generation MAY defer static typing and realize evaluation dynamically,
> provided the deferral is recorded and static typing is realized by a generation derived after the
> seed. Rationale in [spec/learnings/2026-07-02-seed-is-a-dynamic-interpreter.md](./spec/learnings/2026-07-02-seed-is-a-dynamic-interpreter.md).
>
> **Amendment 0.3.0 (2026-07-04).** Core Principle XIV is reframed: the behavioral oracle is the
> executable semantics *as recorded by the conformance corpus* rather than necessarily a reference
> interpreter, and the independence of the judgment is supplied by *two implementations of the
> compiler* (a foreign-language seed compiler and the Cadenza-authored compiler) that must agree,
> rather than by an interpreter-vs-compiler differential. A reference interpreter becomes an optional
> (`MAY`) independent oracle, not a required artifact. This makes the seed a reference *compiler* and
> the runtime uniform, removing a separately-maintained execution engine. Rationale in
> [spec/learnings/2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md](./spec/learnings/2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md).
> This does not touch the never-downgradable Governance Floor (determinism and capability-safety).
>
> **Amendment 0.4.0 (2026-07-04).** Core Principle VII's bootstrap carve-out (Amendment 0.2.0) is
> RETIRED. That carve-out let the seed defer static typing *because it realized evaluation
> dynamically*; under the two-compiler bootstrap (Amendment 0.3.0) the seed compiles to a component
> rather than evaluating, so the dynamic-evaluation basis for the deferral no longer holds. VII now
> requires the seed compiler to reject ill-typed programs with the type rule's machine-readable code,
> realized incrementally over the rules it covers (reject-don't-miscompile), rather than defer typing
> to a later generation. Consequence: a comparison across a nominal boundary, a numeric-type mismatch,
> a non-exhaustive match, a contradicting annotation, and an undeclared capability are compile-time
> *rejections* (the corpus `(compiler …)` clauses), not dynamic traps. Rationale in
> [spec/learnings/2026-07-04-static-typing-is-mandatory-post-pivot.md](./spec/learnings/2026-07-04-static-typing-is-mandatory-post-pivot.md).
>
> **Amendment 0.5.0 (2026-07-05).** Core Principle XI ("Diagnostics Are Machine-Actionable") is
> STRENGTHENED: a diagnostic that reports a rejection must now carry not only a code, a span, and the
> rule it enforces, but a **machine-applicable route to a compliant program** — a structural AST edit —
> and that route must be marked *verified* where the compiler has confirmed by application-and-recompile
> that it clears the diagnostic, or carry an *applicability marker* where the repair is a heuristic. This
> serves the architecture's stated purpose (an agent produces a safe program with no human feedback):
> because the canonical form is the AST and the compiler can apply and recompile its own proposed edit, a
> Cadenza fix can be a verified property rather than a suggestion. The amendment *adds* obligations and
> weakens no governance floor (determinism and capability-safety are untouched), so it needs no
> human-approval floor beyond the operator's decision. Rationale in
> [spec/learnings/2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program.md](./spec/learnings/2026-07-04-a-rejection-carries-a-verified-route-to-a-compliant-program.md).
>
> **Amendment 0.6.0 (2026-07-05).** The compiler emits a program against a single, well-known
> **value-heap runtime** — a shared component the program imports and the host composes — that owns the
> entire storage, layout, reference-count discipline, reclamation, and rendering of the program's runtime
> values; a runtime value crosses as an opaque handle the program never interprets, and a compound result
> is obtained by the host invoking the runtime's render over the returned handle. This carries a Component
> ABI version increment (v2→v3, with a migration path) per the Governance Floor "The Component ABI Changes
> Only By Coordinated Act." It refines — and, by the operator's explicit approval, is permitted to refine —
> the *auditability* of the capability-safety floor: capability-safety was verifiable by counting a
> component's imports, and it is now verifiable as "every import **other than the one well-known runtime
> interface** is a capability the manifest enumerates." The guarantee itself is NOT downgraded — reaching an
> undeclared host operation remains a compile-time rejection that no configuration can reduce to a warning,
> the exemption is a closed allowlist of exactly one interface (not an open class of non-effect imports),
> and the runtime import is neither a host function nor a suspension point. Because it touches a
> never-downgradable floor's audit rule, it required and received explicit human approval per the Amendment
> Discipline. Rationale in
> [spec/learnings/2026-07-05-the-value-heap-runtime-is-a-shared-component.md](./spec/learnings/2026-07-05-the-value-heap-runtime-is-a-shared-component.md).
