# Capability — Capabilities And Effects

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines a program's capabilities as its escaping effect row, the suspend-and-replay boundary
> by which a host call is made, and the intra-program handlers that discharge effects that never escape.
> Requirements realize [Core Principle III](../../constitution.md), [Core Principle IV](../../constitution.md),
> and [Core Principle VIII](../../constitution.md) and trace to [overview §6](../overview.md) and
> [overview §12](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes how a program reaches the world and how effects are structured. The mandatory
floor: a program declares every host capability it requires, reaching an undeclared capability is a
compile-time error, and a program's escaping effect row equals its imported host functions — this is
how "no ambient authority" becomes a property of the program, and it feeds the host-interface-binding
contract that makes the emitted imports mirror the manifest. Every host call is a suspension point: the
program yields to the host, which resolves the call and resumes the run by deterministic replay, so
durable suspend-and-resume and capability-safety are one mechanism. Above this floor, intra-program
effects that never escape to the host are discharged by algebraic handlers, and effect-row *typing* is
an opt-in verification layer that is meaning-preserving. The type-level machinery of the effect row —
that it is a row unified like an open record — is fixed in type-system.md; this document fixes its
behavior.

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

## The Manifest Is The Escaping Effect Row

### A Host Import Is A Boundary Effect And The Manifest Is Its Row

A program's escaping effect row MUST equal the set of host functions it imports, so that a capability and a boundary effect are one concept and the manifest is a projection of that row.

Purity MUST be the empty effect row: a program that imports no host function MUST reach no effect that escapes to the host and MUST run to normal termination without suspending, so that a program's determinism is legible from an empty manifest.

### The Value-Heap Runtime Is The One Import That Is Not A Capability

The single, well-known value-heap runtime interface a program imports to construct and render its runtime values MUST NOT be counted as a host function, so that importing it adds nothing to the escaping effect row and a program that imports only it remains pure with an empty manifest.

Exactly one such runtime interface MUST be exempt — the value-heap runtime the compiler emits programs against, fixed at the declared-default location — and every other import a program carries MUST be treated as a host function and therefore a capability, so that the exemption is a closed allowlist of one and not an open class of non-effect imports.

An import of the value-heap runtime interface MUST NOT be a suspension point and MUST NOT appear in the manifest, so that reaching the runtime is an internal linkage the compiler controls rather than an effect that escapes to the host, and capability-safety stays auditable as "every import other than the one well-known runtime interface is a capability the manifest enumerates."

### Every Host Call Is A Suspension Point

A host call MUST be a suspension point at which the program yields control to the host rather than blocking inside the component, so that resolving a host call is the host's concern and the program holds no in-flight host operation.

The program MUST NOT carry resume state across a suspension: a run MUST be re-invokable from its entry with the same input, so that what advances a run is the responses the host supplies, not state retained in the component.

### Suspension Is Replay From The Host's Log

The host MUST own the ordered log of responses to the host calls a run has made, and MUST resolve a run by re-invoking it from the entry and, at each host call, returning the logged response for that call or — at the first call with no logged response — recording the pending call and suspending the run.

A run's observable behavior MUST be a deterministic function of its input and the responses in that log, so that re-invoking a run with the same input and the same responses reproduces the same host-call sequence and advances to the same point (constitution III).

The response the host supplies for a host call MUST be the value it records in the log for that call, so that a run resumed in place and a run torn down and replayed from the log produce identical observable behavior, and the resumption strategy the host chooses is therefore not part of a program's observable behavior.

### A Durable Continuation Is Canonical Data

A suspended run's continuation MUST be exactly its content-addressed component, its input, and the host's response log — all canonical-form data — rather than a serialized image of the component's memory, so that a suspended run can be resumed on any conforming runtime, including a different host from the one that suspended it.

## Intra-Program Effects Are Handled By Algebraic Handlers

### An Effect That Does Not Escape Is Discharged By A Handler

A program MUST be able to raise an effect operation and discharge it with a handler that establishes a context for the sub-computation it wraps, so that an effect handled entirely within the program is expressible without a host capability.

An effect discharged by an in-program handler MUST NOT appear in the program's manifest, so that only effects that escape to the host — those no handler discharges — are capabilities.

Mutation MUST be expressible as a state effect discharged by a handler that threads state purely, so that a program can express mutable-looking computation without the value heap becoming mutable.

### Handler Resolution Is Lexical And Deterministic

A raised effect operation MUST be discharged by the nearest enclosing handler for that operation, resolved lexically at compile time, so that which handler discharges an operation is a deterministic function of the source (constitution III).

### A Continuation Is One-Shot By Default

A handler MUST resume the continuation of an effect operation at most once by default, so that a suspended computation and the resources it holds are not duplicated and fuel accounting and reference counting stay sound.

A handler that resumes a continuation more than once MUST be admitted only where a build's declared defaults enable multi-shot resumption, so that the default keeps the affine discipline and multi-shot is a deliberate opt-in.

### Capabilities Attenuate: A Handler Forwards A Narrower Row

A handler MUST be able to grant its sub-computation a subset of the effect row it holds, so that a caller can pass a callee fewer capabilities than it has itself.

A handler MUST NOT grant its sub-computation an effect row label it does not itself hold, so that attenuation only ever narrows authority and never widens it, making "no ambient authority" transitive across a call.

The subset relationship between a forwarded row and the row a handler holds MUST be checked at compile time, so that an over-broad forward is a compile-time rejection rather than a runtime failure.

## Effect-Row Typing Is An Opt-In Layer

### Effect-Row Annotation Is Opt-In

A program MUST compile without any effect-row annotation, the compiler inferring each function's effect row from the operations it reaches, so that the mandatory floor is the escaping row itself, not a written annotation.

A program MAY annotate a function with the effect row it performs.

### An Annotated Effect Row Is Checked

When a function carries an effect-row annotation, the compiler MUST reject the program if the function reaches an effect the annotation's row does not permit.

When a function carries an effect-row annotation, the compiler MUST reject the program if a caller does not account for the row the function declares.

### The Annotation Layer Preserves Meaning

Adding an effect-row annotation to a program that already compiles MUST NOT change the program's runtime meaning, so that annotating effects is a verification layer over the operational effects that already run, not a change to them.

## Optionality

### Effect-Row Typing Is Optional

Effect-row typing MUST be an optional capability that a build may include or exclude, in accordance with the build's declared defaults, while the mandatory capability declaration and the suspend-and-replay boundary remain part of the floor regardless.

### The Declared Default Is Include

When a build is not told whether to include effect tracking, it MUST include it.
