# Frozen Contract — Component ABI

> **FROZEN CONTRACT.** This document pins how Cadenza's type universe maps onto the host
> interface's type system, the calling convention across the component boundary, and the memory
> layout of values that cross it. It is the byte-level agreement that lets a component derived by
> one generation of the compiler interoperate with a component derived by another. It is versioned
> and changed only by the coordinated act described in the constitution's Governance Floors. Its
> requirements realize [Core Principle VI](../../constitution.md) and
> [Core Principle VII](../../constitution.md) and trace to [overview §5](../overview.md),
> [overview §6](../overview.md), and [overview §8](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract states the mapping's properties; the concrete type table that
> realizes it is pinned at the declared-default location.
>
> **Contract version: 2.** Version 2 adds the entry's suspension outcome (§The Entry May Suspend On A
> Host Call): the entry's result distinguishes normal completion, a trap, and a suspension carrying the
> pending host call, so that a host call is a suspension point resolved by replay
> (capabilities-and-effects.md §Every Host Call Is A Suspension Point). **Migration:** the suspension
> outcome is a new arm of the entry's result rather than a change to the normal-completion or trap
> representation, so a component that reaches no host function crosses the boundary exactly as under
> version 1; the increment is recorded because the entry's result type gains an arm. This precedes any
> deployed component, so no in-the-wild artifact requires re-derivation.
>
> **Contract version: 3.** Version 3 adds the value-heap runtime import (§The Value-Heap Runtime Crosses
> By A Well-Known Import): a derived program imports the single, well-known value-heap runtime interface
> to construct and inspect its runtime values, and a program's runtime values live in that runtime's
> linear memory and cross the internal runtime boundary as opaque handles, so the compiler emits programs
> against a shared runtime rather than open-coding a value heap into every component. The runtime stores
> and reclaims values; it does not name or render them — a value's field and variant names are compile-time
> knowledge the runtime does not hold (a record is a positional product at run time), so rendering a value
> to its canonical text is type-directed code the compiler emits into the program itself, and a compound
> result therefore crosses the boundary as an ordinary string the program returns (§A Compound Result Is
> Rendered By Compiler-Emitted Code). This runtime import is not a host function and not a capability
> (capabilities-and-effects.md §The Value-Heap Runtime Is The One Import That Is Not A Capability).
> **Migration:** the runtime import is a new, closed boundary — a program that produces only scalar/unit
> results and imports neither a host function nor the runtime crosses exactly as under version 2 — and the
> compound-result output convention changes from a component-owned `display()` resource to an ordinary
> string result the program produces by walking the value through the runtime's accessors; this precedes
> any deployed compound-returning component, so no in-the-wild artifact requires re-derivation.
>
> **Contract version 3, refinement.** Clarifies §The Runtime Does Not Name Or Render Values: the runtime
> is not only name-free but TAG-FREE — it holds no per-value type identity, only structure and data (a
> product's elements, a sum's variant discriminant, a leaf's payload). Because the language has no type
> erasure, a reader (the compiler-emitted renderer, a consumer) always knows a value's static type and
> never dispatches on a runtime tag. This narrows, not widens, the runtime's obligations, and it is a
> representation concern behind the opaque handle, so no artifact requires re-derivation. **Migration:**
> none — a runtime that carried a type tag internally would still satisfy the interface; the requirement
> only forbids the interface from EXPOSING a type identity, which no version-3 interface did.
>
> **Contract version: 4.** Version 4 RETRACTS version 2's suspension outcome entirely and stops encoding
> a trap as a result arm: the component's exported entry is a **plain function `input -> output`** whose
> signature carries the program's declared result type and nothing else — no suspension arm, no injected
> trap arm, no resume parameter (§The Entry Is A Plain Function). Two consequences: (1) A host call is an
> ordinary imported-function call that returns its response; how the host resolves it — synchronously, by
> suspending a fiber and resuming in place, or by tearing the instance down and replaying from a response
> log — is host runtime policy the ABI does not encode, the language's only cross-boundary requirement
> being determinism (capabilities-and-effects.md §A Host Call Returns A Response; §A Run Is A Deterministic
> Function Of Its Input And Responses). (2) A trap remains the internal halt for unexpected conditions
> (division by zero, `expect` on absence, a host function that aborts the run), but it is wasm's
> **out-of-band** halt the embedder observes when it invokes the entry — not a variant the component's
> interface declares, where it would be redundant with that ambient channel. **A host-delegated effect
> therefore appears as its WIT import contract verbatim:** an operation `(op nm (-> P… R))` becomes exactly
> `nm: func(p: P…) -> R` in an imported interface, with the compiler injecting no error arm, no state, and
> no extra parameter (§A Host-Delegated Operation Imports Verbatim). If an operation's own declared result
> is fallible, that fallibility is in the operation's declared type and the program handles it — it is not
> something the boundary adds. The v2 rationale (program carries no resume state, re-invokable from entry)
> described one host strategy and is contradicted by fibers (which freeze the wasm stack, so the stack IS
> the resume state); the language mandates neither. **Migration:** the entry's result loses the suspension
> and trap arms, a boundary-representation change carrying a version increment; but both preceded any
> deployed component, so no in-the-wild artifact requires re-derivation. A program that reaches no host
> function keeps a plain `input -> output` entry, byte-identical to v1's normal-completion representation.
>
> **Contract version: 5.** Version 5 adds the cross-component shared-runtime value transport (§Cadenza
> Components Composed Against A Shared Runtime Exchange Values As Handles): when two separately-derived
> Cadenza components are composed against one value-heap runtime instance, a compound value passes between
> them as an opaque handle into that shared runtime (its concrete boundary form fixed at the declared-default
> location), rather than being marshaled into a component-model aggregate at each hop, so that a value crosses between Cadenza
> components with no serialization while remaining meaningful only within the one shared runtime instance
> that owns it (§The Value-Heap Runtime, §A Runtime Value Crosses As An Opaque Handle). This is the internal
> composition representation for a value crossing between Cadenza components; the external marshaling
> representation (the type table at the declared-default location) is unchanged and remains how a value
> crosses to a non-Cadenza consumer. **Migration:** the shared-runtime handle transport is a new boundary
> representation for a mode that previously had none — before this version no two Cadenza program components
> were composed against one runtime, so no value crossed between them at all — and it is therefore additive
> (§Additive Evolution): a single-program run, and a value crossing to a non-Cadenza consumer by marshaling,
> are both byte-identical to version 4. No in-the-wild artifact requires re-derivation.

## Purpose And Scope

Every value that enters or leaves a component crosses a boundary between Cadenza's types and the
host interface's types. For a component to be a stable, content-addressed artifact that outlives
the compiler that produced it, that crossing must be fixed: the same exported signature must lower
and lift the same bytes regardless of which compiler generation emitted it. This contract pins the
type mapping, the calling convention, the boundary layout, and the component's entry — the export
through which it is invoked. It does not pin the internal representation a component uses for values
that never cross a boundary, which the compiler is free to choose, nor the concrete entry signature
of each program shape, which is a declared-default choice.

## The Boundary Type Mapping

### Every Exported Type Has A Stable Boundary Representation

Each Cadenza type that may appear in an exported or imported signature MUST have a single stable representation in the host interface's type system.

A type that has no defined boundary representation MUST NOT appear in an exported or imported signature.

The boundary representation of a type MUST be a function of the type alone, independent of the compiler generation that emits it.

### Generics Do Not Cross The Boundary

The compiler MUST monomorphize every exported and imported signature to concrete types before emitting the component interface.

A generic definition MUST NOT appear in a component's interface.

## The Calling Convention

### Lowering And Lifting Are Fixed Inverses

The lowering of a Cadenza value to its boundary representation MUST be a total function fixed by this contract.

The lifting of a boundary representation to a Cadenza value MUST be the inverse of the pinned lowering for every value in the lowering's range.

The calling convention across the boundary MUST be a function of the declared signature alone, independent of compiler internals.

## The Component Entry

### A Component Exports A Defined Entry

A derived component MUST export an entry through which the runtime invokes it.

The compiler MUST determine a program's entry from the program rather than leave it implicit.

### The Entry Signature Crosses The Boundary By The Same Rules

The entry's parameter and result types MUST each have a boundary representation fixed by this contract.

The entry's input and output MUST lower and lift across the boundary by the same calling convention as any other boundary value.

### The Entry Defines The Input For Oracle Agreement

The input over which a compiled program's behavior is compared to the recorded corpus semantics MUST be the input to the program's entry.

### The Entry Is A Plain Function

The entry's exported signature MUST be a plain function from the program's input type to its result type, carrying no additional outcome arm — no suspension outcome and no injected trap outcome — so that a run either returns its result value or halts out-of-band, and the interface declares nothing beyond `input -> output`.

A trap MUST be an out-of-band halt the embedder observes when it invokes the entry — the wasm-level failure a partial operation or an aborting host function raises — rather than a variant the entry's result type declares, so that the internal trap mechanism (core-semantics.md §A Trap Halts Execution At A Defined Point) stays a run's terminal behavior and is not duplicated as a redundant arm of the interface.

The entry MUST NOT carry a resume parameter and its result MUST NOT encode a pending host call or a position in the program's execution, so that how a host call suspends and resumes is host runtime policy the ABI does not represent (capabilities-and-effects.md §A Host Call Returns A Response) and the same emitted bytes serve a host that answers inline, one that suspends a fiber and resumes in place, and one that tears down and replays from a log.

The host MUST NOT require the component to encode any resume state, so that whichever resumption strategy a host chooses is invisible to the emitted component and constrained only by the run's determinism (capabilities-and-effects.md §A Run Is A Deterministic Function Of Its Input And Responses).

## Boundary Memory Layout

### Aggregate Layout Is Determined By Type

The byte layout of an aggregate value that crosses the boundary MUST be determined solely by its declared type.

The byte layout of an aggregate value that crosses the boundary MUST NOT depend on the order in which the compiler discovered or emitted its fields.

Padding and alignment inserted into a boundary aggregate MUST be a fixed function of the aggregate's declared type.

## The Value-Heap Runtime

### The Value-Heap Runtime Crosses By A Well-Known Import

A derived program MUST reach its runtime values — constructing a compound value and inspecting a value's contents — through the single, well-known value-heap runtime interface it imports, rather than by open-coding a value heap into its own component, so that the heap representation is one shared artifact the compiler emits programs against.

The identity of that runtime interface MUST be fixed at the declared-default location and MUST be the same for every program a generation emits, so that any conforming host can satisfy the import and the interface is a stable part of the ABI rather than a per-program choice.

The concrete runtime a program is emitted against MUST be identified by the content address of that runtime component, so that a program's execution is deterministic in the pair (program, runtime content address) and a runtime built from different bytes is a distinct, explicitly-identified environment rather than a silent substitution (reproducible-derivation.md §Derivation Is A Function Of Source And Toolchain).

### The Emitted Component Records Its Required Runtime

A compiler MUST be built against a fixed runtime interface and a fixed runtime content address, so that which runtime a generation targets is a property of the compiler rather than a per-invocation choice, and the compiler and its runtime are one versioned pair.

A derived program MUST record, in the emitted component itself, the content address of the runtime it requires, so that the component is self-describing: what interface it imports and which exact runtime implementation satisfies that import both travel with the artifact.

### The Host Resolves The Runtime By Content Address

A host MUST resolve a program's runtime import by reading the required runtime content address the component records and locating the runtime component of that content address in a content-addressed store, rather than by assuming a single ambient runtime, so that programs pinned to different runtime versions coexist and each resolves the exact runtime it was emitted against.

A host that cannot locate a runtime of the content address a component requires MUST refuse to run the component rather than substitute a different runtime, so that a mismatched runtime is a detected error rather than a silent change in observable behavior.

### The Runtime Owns The Value Heap And Its Representation

The value-heap runtime MUST own the entire storage of a program's runtime values — their allocation, their in-memory layout, their reference-count discipline, and their reclamation — so that a program component holds no value storage of its own and the representation of every compound value is the runtime's private concern.

The internal representation a value has within the runtime MUST NOT be observable across the runtime boundary, so that the runtime may change how it lays out, shares, counts, or reclaims a value without altering any program's observable behavior or requiring a program to be re-derived.

The runtime MUST expose the operations that construct a compound value from its parts and that read a component out of a compound value by position, so that a program builds and takes apart its values entirely through the interface and never by reaching into the runtime's memory.

### The Runtime Does Not Name Or Render Values

The value-heap runtime MUST NOT hold the field names of a record, the variant names of a sum, or any other source-level name of a value, so that a record is a positional product and a sum is a discriminated payload at run time and the association of a position with a name is compile-time knowledge the runtime does not carry.

The value-heap runtime MUST NOT hold a value's TYPE as a per-value tag, so that — because the language has no type erasure and the compiler therefore knows a value's static type at every use site — the runtime stores only structure and data (a product's elements, a sum's variant discriminant, a leaf's payload) and never a type identity a reader would dispatch on. The variant discriminant a sum carries is the runtime datum recording WHICH variant a value is, not the sum's type; the compiler maps a discriminant to a variant name.

The value-heap runtime MUST NOT render a value to its canonical text, so that rendering — which requires the names and the type the runtime does not hold — is type-directed code the compiler emits (walking a value of statically-known shape through the runtime's accessors) rather than a service the runtime provides.

### A Runtime Value Crosses As An Opaque Handle

A runtime value that crosses between a program and the value-heap runtime MUST cross as an opaque handle whose interpretation belongs solely to the runtime, so that the value's byte representation is the runtime's internal concern and never a layout the program or the host depends on.

The program MUST NOT dereference or interpret a runtime handle, so that the acyclic reference-counted heap the runtime owns is not aliased by another linear memory and the handle is a capability-free token rather than a pointer into shared state.

A runtime handle MUST be meaningful only within the single run and runtime instance that produced it, so that a handle never escapes the run that produced it and a host that resumes a run by replaying it reconstructs the run's values through the runtime rather than by carrying a handle across the boundary (the handle is not durable state the ABI transports; whether and how a host replays is host policy — capabilities-and-effects.md §A Run Is A Deterministic Function Of Its Input And Responses).

### A Compound Result Is Rendered By Compiler-Emitted Code

The observable result of a program that produces a compound value MUST be an ordinary string the program returns, produced by type-directed code the compiler emits that walks the value through the runtime's accessors and assembles its canonical text, rather than by the runtime rendering the value or by the program's own component owning a display of it, so that the names a rendering requires stay with the compiler that holds them and the host reads back a plain string (host-interface-binding.md §The Host Does Not Format A Component's Values).

The text the compiler-emitted rendering produces MUST be the value's canonical text form under deterministic-value-form.md, so that a compound result crossing the boundary is byte-identical to the same value's recorded corpus form.

## Cross-Component Value Exchange

### Cadenza Components Composed Against A Shared Runtime Exchange Values As Handles

Two or more separately-derived Cadenza components that a host composes against a single value-heap runtime instance MUST exchange a compound value that crosses between them as an opaque handle into that shared runtime, rather than by marshaling the value into a component-model aggregate at the crossing, so that a value passes between Cadenza components with no serialization and the shared runtime that owns the value is the one place its representation lives.

A scalar value that crosses between such components MUST cross by its component-model scalar representation and not as a handle, so that only a value the runtime owns is carried by handle and a scalar carries no runtime dependency.

The opaque handle by which a compound value crosses MUST be interpretable only by the shared runtime — the same runtime handle a program exchanges with the runtime across its internal boundary (§A Runtime Value Crosses As An Opaque Handle) — so that a handle one component produces is a value the other accepts without either dereferencing it, and the concrete boundary form of that handle (a runtime handle valtype, or a well-known `value` resource type the runtime interface publishes) is fixed at the declared-default location rather than by this contract.

### A Cross-Component Handle Is Meaningful Only In The Shared Runtime Instance

A `value` handle that crosses between composed components MUST be meaningful only within the single runtime instance the composition shares, consistent with a runtime handle being meaningful only within the instance that produced it (§A Runtime Value Crosses As An Opaque Handle), so that composing components against a shared runtime is what makes a handle one produces intelligible to another and a handle never denotes a value in a different instance.

A host that composes Cadenza components which exchange values by handle MUST bind every such component's value-heap runtime import to the one shared runtime instance, so that the components' handles index one heap and a component cannot be handed a handle into a heap it does not share.

A composed component MUST NOT dereference or interpret a handle it receives from a peer, reading the value only through the shared runtime's accessors as the value's statically-known type, so that the receiving component depends on the runtime's interface rather than on the handle's byte representation and no peer's heap is aliased by another linear memory.

### The Exchanged Signature Is Monomorphic

A cross-component imported or exported signature by which components exchange values MUST be monomorphic, per §Generics Do Not Cross The Boundary, so that the exchanged interface names concrete types and a component binds a peer's export at a fixed instantiation the peer emitted rather than requesting an instantiation on demand.

### Ownership Of An Exchanged Handle Follows The Crossing Direction

A `value` handle a component passes as an argument to a peer MUST cross as a borrow that the receiving component reads but does not reclaim, so that the passing component retains ownership and reclaims the value, matching the repeated-use callback shape a borrowed handle already serves.

A `value` handle a component returns as a result to a peer MUST transfer ownership to the receiving component, which later reclaims it through the shared runtime, so that a returned value's storage is balanced by exactly one reclamation and no value the runtime owns is leaked or double-freed across the crossing.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract that alters the boundary representation, calling convention, or layout of an already-defined type MUST carry a version increment, per the constitution's Governance Floor on the component ABI.

A change to this contract that alters the boundary representation, calling convention, or layout of an already-defined type MUST carry a stated migration path, per the constitution's Governance Floor on the component ABI.

A change to this contract that only adds a boundary representation for a type that previously had none MUST be permitted as an additive change.
