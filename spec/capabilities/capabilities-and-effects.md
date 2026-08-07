# Capability — Capabilities And Effects

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines a program's capabilities as its escaping effect row, the deterministic host-call boundary
> by which a host call is made, and the intra-program handlers that discharge effects that never escape.
> Requirements realize [Core Principle III](../../constitution.md), [Core Principle IV](../../constitution.md),
> and [Core Principle VIII](../../constitution.md) and trace to [overview §6](../overview.md) and
> [overview §12](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes how a program reaches the world and how effects are structured. The mandatory
floor: an effect declaration is a routing-agnostic contract, each entrypoint delegates to the host the
effects it grants boundary access, reaching an effect neither handled nor delegated is a compile-time
error, and a program's escaping effect row equals the effects its entrypoints delegate — this is how
"no ambient authority" becomes a property of the program, and it feeds the host-interface-binding
contract that makes the emitted imports mirror the manifest. A host call is an ordinary call to an
imported function that returns its response; a run is a deterministic function of its input and the
ordered responses to its host calls, and how the host resolves a call — inline, by suspending and
resuming in place, or by re-deriving the run — is host policy the language does not prescribe. Above
this floor, effects that never reach a delegation are discharged by algebraic handlers, and effect-row
*typing* is an opt-in verification layer that is meaning-preserving. The type-level machinery of the effect row —
that it is a row unified like an open record — is fixed in type-system.md; this document fixes its
behavior.

## Capability Declaration

### An Entrypoint Delegates The Capabilities It Grants To The Host

An entrypoint MUST enumerate, at the entrypoint itself, every effect whose operations it delegates to the host, so that granting a capability is a decision made where authority enters the program rather than a property an effect's declaration carries.

Declaring an effect and its operations MUST NOT by itself grant any host capability: an effect declaration is a routing-agnostic contract, and only an entrypoint's delegation routes an effect's operations to the host, so that a library that declares or performs an effect cannot enlarge the authority of a program that uses it.

An effect an entrypoint reaches but neither handles in-program nor delegates to the host MUST be treated as not granted.

### Undeclared Capability Is A Compile-Time Error

A program that reaches an effect operation that no enclosing handler discharges and that its entrypoint does not delegate to the host MUST be rejected at compile time, so that every effect is either handled in-program or explicitly granted to the host and none is silently ambient.

The compiler MUST determine a program's required capabilities from the operations its entrypoints actually reach and delegate, rather than from a separately-asserted list that could understate them.

### The Program Manifest Is The Union Of Its Entrypoints' Delegations

A program's capability manifest MUST be the union of the host delegations its entrypoints declare, so that the manifest is a projection of where authority actually enters the program and not of every effect any module declares.

Dependency resolution MUST NOT introduce a capability that no entrypoint in the program delegated.

## The Manifest Is The Escaping Effect Row

### A Host Import Is A Boundary Effect And The Manifest Is Its Row

A program's escaping effect row MUST equal the set of effects its entrypoints delegate to the host, so that a capability and a boundary effect are one concept and the manifest is a projection of the effects an entrypoint routes to the boundary rather than of every effect declared.

Purity MUST be the empty effect row: an entrypoint that delegates no effect to the host MUST reach no effect that escapes and MUST run to normal termination without suspending, so that an entrypoint's determinism is legible from an empty delegation and an entrypoint whose every reached effect is handled in-program is pure.

### The Value-Heap Runtime Is The One Import That Is Not A Capability

The single, well-known value-heap runtime interface a program imports to construct and inspect its runtime values MUST NOT be counted as a host function, so that importing it adds nothing to the escaping effect row and a program that imports only it remains pure with an empty manifest.

Exactly one such runtime interface MUST be exempt — the value-heap runtime the compiler emits programs against, fixed at the declared-default location — and every other import a program carries MUST be treated as a host function and therefore a capability, so that the exemption is a closed allowlist of one and not an open class of non-effect imports.

An import of the value-heap runtime interface MUST NOT be a host capability and MUST NOT appear in the manifest, so that reaching the runtime is an internal linkage the compiler controls rather than an effect that escapes to the host, and capability-safety stays auditable as "every import other than the one well-known runtime interface is a capability the manifest enumerates."

### A Host Call Returns A Response

A host call MUST be an ordinary call to an imported function that returns its response to the program, so that from the program's side reaching the host is a plain function call and how the host produces the response is the host's concern.

The program MUST NOT observe or encode how a host call is resolved, so that whether the host answers inline, suspends the run and resumes it later, or aborts it is invisible to the program and not part of the program's meaning.

### A Run Is A Deterministic Function Of Its Input And Responses

A run's observable behavior MUST be a deterministic function of its input and the ordered responses to the host calls it makes, so that the same input and the same responses in the same order reproduce the same host-call sequence and the same result (constitution III).

This determinism MUST be the language's only requirement on the host boundary: the language MUST NOT mandate how a host suspends, resumes, or resolves a call, so that a host is free to answer synchronously, to suspend and resume a run in place, or to tear a run down and re-derive it, and every faithful strategy produces identical observable behavior.

### How A Host Resumes Is Host Policy, Not Language

The mechanism by which a host resolves a call it cannot answer immediately — suspending an in-memory fiber and resuming in place, or discarding the run and re-deriving it from the ordered responses it has recorded — MUST be host runtime policy the language neither prescribes nor represents, so that portable re-derivation and local fiber suspension are both admissible and the emitted component is identical under either.

Because a host MAY choose to re-derive a run from its input and recorded responses, a program that is a deterministic function of those (the requirement above) MUST remain resumable under that strategy without carrying any resume state itself; but a host that instead suspends the run in place MAY hold the run's live state, so the language requires determinism rather than statelessness and leaves the choice to the host.

## Authority Is Acknowledged Per Entrypoint And Bound Per Component

### Each Entrypoint Acknowledges Its Own Escaping Row

Each entrypoint MUST acknowledge, at itself, the effects it delegates to the host, so that the authority a given entrypoint is permitted to reach is a property of that entrypoint and not of the module or component that contains it, and an entrypoint that delegates nothing is pure regardless of what its neighbors delegate.

The authority an entrypoint reaches MUST be determined by the operations reachable from its own body under its own delegations, so that co-locating a pure entrypoint with an effectful one in the same component does not grant the pure entrypoint any authority.

### A Component Is Bound Against The Union Of Its Entrypoints' Rows

The set of host operations a component imports MUST be the union of the escaping rows its entrypoints acknowledge, so that a component instantiated once carries a single import surface serving every entrypoint it exports, as the component model requires.

The host grant required to instantiate a component MUST be that union, so that provisioning is per-component even though acknowledgment is per-entrypoint, and an entrypoint that reaches fewer effects than its neighbors is still hosted in a component provisioned for all of them.

### Authority Availability Is Not Authority

A host operation a component imports for one entrypoint's sake MUST NOT be reachable by another entrypoint that does not itself reach it, so that an import present in the instance for one export is inert for an export whose body never performs it — availability in the instance is not authority in the call graph.

Authority MUST NOT be a runtime value that a program can store, pass, or return, so that no entrypoint can obtain authority through shared linear memory — memory carries data, never capability — and per-entrypoint authority stays sound when entrypoints share an instance.

### Per-Entrypoint Grant Isolation Is A Decomposition

Isolating one entrypoint so the host provisions strictly fewer capabilities for it than for another MUST be achieved by placing them in separate components, so that per-entrypoint acknowledgment is a source-and-type property while a strictly smaller *grant* is obtained by component decomposition rather than expected to follow from acknowledgment within one component.

## An Effect Is Declared With Its Operations

### An Effect Declaration Names The Effect And Types Its Operations

A program MUST be able to declare an effect that names it and binds each of its operations to an operation type, so that the set of operations an effect offers is a closed, statically-known set rather than an open collection of ad-hoc names.

An operation MUST be reached through its declaring effect, so that two effects may each declare an operation of the same name without collision and the effect an operation belongs to is unambiguous at every performance and every handler arm.

The concrete form by which an effect and its operations are declared MUST be pinned at the declared-default location, so that two builds agree on the surface an effect declaration takes.

### Performing An Operation Is Typed And Contributes To The Row

Performing an operation MUST check its arguments against the operation's declared parameter types and yield the operation's declared result type, so that an effect operation is typed exactly as an ordinary function application is.

Performing an operation MUST add its declaring effect to the effect row of the function that performs it, so that a function's inferred row is the set of effects its operations reach and the manifest of delegated effects is a projection of that row.

### A Guard Is Side-Effect-Free

An effect operation performed in a match-arm guard MUST be rejected at compile time, so that a guard is a pure decision the pattern engine may evaluate speculatively or repeatedly without observable effect.

### A Handler Arm Names An Operation Its Effect Declares

A handler arm that names an operation the arm's effect does not declare MUST be rejected at compile time, so that a handler discharges only operations that exist and the declaration remains the closed set of an effect's operations.

### A Handler Discharges Exactly One Effect

A handler MUST discharge exactly one effect — every arm of a single handler names an operation of the same declaring effect — so that a handler installs one effect's context and the effect a handler discharges is unambiguous, mirroring that an operation is reached through its declaring effect.

Discharging several effects over one sub-computation MUST be expressed by nesting a handler per effect, so that each handler in the nest discharges its own single effect and no handler mixes the operations of two effects, keeping the discharged effect a property of the handler rather than an open collection its arms enumerate.

### A Handler Discharges Its Effect

A handler MUST bind every operation its effect declares, so that installing a handler for an effect discharges the whole of that effect's closed operation set — the effect analogue of a match covering every variant of its scrutinee's sum — and no operation of the effect a handler claims to discharge is left without a discharger under that handler.

A handler that omits an operation its effect declares MUST be rejected at compile time, so that a partially-handled effect is a compile-time error rather than an operation that silently escapes the handler that appears to discharge it, and the rejection SHOULD identify the omitted operations so the gap is mechanically repairable.

## An Effect Is Routed By A Handler Or By Host Delegation

### Host-Binding Is A Routing Decision Made At The Entrypoint

An effect declaration MUST NOT determine whether the effect is discharged in-program or at the host boundary, so that an effect is a routing-agnostic contract and the same declared effect may be handled in one program and delegated to the host in another.

An entrypoint MUST be able to delegate a set of effects to the host boundary, fixing that within the delegated computation those effects are discharged at the component boundary by an imported-function call the host resolves, so that the host is the *terminal* handler of a delegated effect and delegation is the boundary counterpart of an in-program handler.

An effect an enclosing handler discharges MUST NOT appear in the manifest, and an effect an entrypoint delegates to the host MUST be enumerated in the program's manifest and reached there as a call to an imported host function, so that whether a given performance escapes is determined by the handlers dynamically enclosing it and the delegation enclosing it, and a delegated effect always has exactly one terminal discharger — the host.

The concrete form by which an entrypoint delegates a set of effects to the host MUST be pinned at the declared-default location and MUST resolve an operation it delegates exactly as the nearest enclosing handler would, so that host delegation is the boundary member of the same nearest-enclosing resolution as in-program handling and two builds agree on the surface a delegation takes.

### Host Delegation Is An Entrypoint's Prerogative

Only an entrypoint MUST be able to delegate an effect to the host, so that authority enters a program from the top and no interior function can route an effect to the boundary, keeping "no ambient authority" transitive: a library performs and handles effects but never grants host access.

A delegation that names an effect the delegated computation never reaches MUST be rejected at compile time, so that a manifest carries no latent authority — a granted capability that is never exercised — and the manifest is exactly the effects that escape, no more and no fewer.

### An Ungranted Effect Is A Compile-Time Error

An operation performed at a point that has neither an enclosing handler for its effect nor an enclosing host delegation of its effect MUST be rejected at compile time, so that an effect is always either discharged by a handler or delegated to the host and never silently ambient, making "no ambient authority" a compile-time property.

This single check MUST subsume both the reached-but-undelegated host operation and the undischarged intra-program effect, so that the two are one condition — an effect reached an entrypoint's top with no home — rather than two separate diagnostics keyed on a declaration-time host/intra distinction the effect no longer carries.

## Intra-Program Effects Are Handled By Algebraic Handlers

### An Effect That Does Not Escape Is Discharged By A Handler

A program MUST be able to raise an effect operation and discharge it with a handler that establishes a context for the sub-computation it wraps, so that an effect handled entirely within the program is expressible without a host capability.

An effect discharged by an in-program handler MUST NOT appear in the program's manifest, so that only effects that escape to the host — those an entrypoint delegates and no nearer handler discharges — are capabilities.

### A Handler Threads State Across The Operations It Discharges

A handler MUST establish an initial state for the sub-computation it wraps, fixed where the handler is installed, so that a handler that carries state receives its seed explicitly and no state is ambient.

Discharging an operation MUST produce both the value delivered to the point that performed the operation and the next state carried forward to the rest of the sub-computation, so that a handler folds a state across the sequence of operations its body performs and threads it purely, without the value heap becoming mutable.

A handler MUST evaluate to the value of its body, with the state it accumulated observable only through the operations the effect declares, so that reading the accumulated state is the same mechanism as any other operation and a handler needs no separate result form. Mutation is the instance of this that reads and updates the threaded state, so that a program expresses mutable-looking computation without the value heap becoming mutable.

### Handler Resolution Is Dynamic In Extent And Statically Determined

A raised effect operation MUST be discharged by the nearest handler enclosing it in dynamic extent — the nearest handler active along the run's call chain, not the nearest handler lexically enclosing the performing function's definition — so that a function may perform an operation its caller discharges and the same function called under two different handlers is discharged by each in turn.

Which handler discharges each performance MUST be determined statically at compile time by monomorphizing the enclosing handler context over the closed effect row, so that handler resolution is dynamic in extent yet a deterministic function of the source (constitution III) with no runtime handler search.

### A Continuation Is One-Shot By Default

A handler MUST resume the continuation of an effect operation at most once by default, so that a suspended computation and the resources it holds are not duplicated and fuel accounting and reference counting stay sound.

A handler that resumes a continuation more than once MUST be admitted only where a build's declared defaults enable multi-shot resumption, so that the default keeps the affine discipline and multi-shot is a deliberate opt-in.

### A Guard Is Side-Effect-Free

An effect operation performed in a match-arm guard MUST be rejected at compile time, so that a guard is a pure decision the pattern engine may evaluate speculatively or repeatedly without observable effect.

A guard condition is a boolean that refines a pattern: it gates the arm but does not necessarily run — a guard that fails falls through to the next arm, and the pattern engine may evaluate a guard speculatively, out of source order, or more than once. An effect performed in that position therefore has no well-defined execution count or order — it would perform zero, one, or several times depending on the match strategy — so it MUST be a compile-time error rather than a computation with an unspecified effect schedule. The rejection is unconditional: the prohibition is on the guard *position*, not on which effect is performed, so an effect that has a discharging handler in scope is rejected just as one that does not — the defect is where the effect sits, not whether it has a home.

A program that must consult an effect to decide an arm MUST perform that effect once before the `match` — binding its result to a `let` — and guard on the bound pure value, so that the effect has a single well-defined execution and the guard stays a pure decision over its result.

### A Handler May Interpose On An Effect An Entrypoint Would Delegate

A program MUST be able to enclose, in a handler that discharges its operations, an effect an entrypoint would otherwise delegate to the host, so that the operation resolves to that handler rather than reaching the boundary, making it possible to observe, mock, cache, or otherwise stand in for a host capability without the performing code being aware — a handler nearer the perform wins over the delegation that encloses it.

A handler arm that re-performs the operation it is discharging MUST resolve that re-performance against the handlers and delegations enclosing the handler's own declaration, not against the handler itself, so that an arm forwards to the next-outer handler — up to and including the host delegation at the entrypoint — rather than recursing into itself.

An effect that an enclosing handler fully discharges without re-performing it MUST NOT appear in the manifest and MUST NOT reach the boundary, so that an entrypoint whose every otherwise-delegated effect is interposed by a handler is pure with an empty manifest — the mechanism a test harness uses to run an I/O program as a deterministic one.

A side effect that an interposing handler's arm itself performs MUST be replay-idempotent — either an intra-program effect re-derived within the run or a host effect the host makes idempotent — so that if the host resolves a delegated call by re-deriving the run, the interposing arm re-executing during re-derivation reproduces identical observable behavior.

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

Effect-row typing MUST be an optional capability that a build may include or exclude, in accordance with the build's declared defaults, while the mandatory capability declaration and the deterministic host-call boundary remain part of the floor regardless.

### The Declared Default Is Include

When a build is not told whether to include effect tracking, it MUST include it.
