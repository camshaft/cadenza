# Backends-And-Targets Architecture

> **NORMATIVE — REFERENCE BACKEND ARCHITECTURE.** This document prescribes the seam between the
> target-neutral front of the reference compiler and its target-specific back: that a backend is a function
> of the typed core representation and a target-neutral boundary layout, that the flat instruction rung is a
> property of one backend rather than a shared pipeline rung, and what a second backend may and may not
> claim. Its RFC-2119 requirements bind a compiler built to the Cadenza *reference architecture* and are
> citable by the requirement gate for such a compiler.
>
> **This document refines [reference-compiler.md §Instruction Selection Emits Against A Fixed Runtime And
> Envelope](./reference-compiler.md).** That section describes instruction selection for the component
> target — the flat instruction rung, the fixed runtime, the component envelope. This document fixes the
> more general fact that section is one instance of: the pipeline is target-neutral up to a single seam, and
> the flat instruction rung and everything below it is *a* backend, not *the* pipeline. It also refines
> [intermediate-representations.md](./intermediate-representations.md): the "flat sequence below the core"
> that document describes is what a *linearizing* backend produces, and a backend that emits a structured
> target consumes the typed structured core directly and never builds the flat rung at all.
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying exactly
> one obligation, under a stable heading. Per [constitution §XIII](../../constitution.md), the requirements
> below name no concrete engine, prior prototype, target language, or source path; the descriptive lead-ins
> and the learning they cite carry the concrete grounding.

## Purpose And Scope

[reference-compiler.md](./reference-compiler.md) describes a pipeline that ends in a component: instruction
selection lowers the core to a flat instruction rung and a serializer lays it into a component envelope. But
nothing above instruction selection is specific to that target — name resolution, inference, the compile-time
evaluator, the poison and erasure checks, and the boundary layout are all computed the same way whatever the
output is. Fixing where the target-neutral front ends and the target-specific back begins lets the same front
serve more than one backend — a component for a host boundary, or source in another language linked as an
ordinary library — without duplicating or destabilizing the front. This document fixes that seam. It does not
add a second backend; it fixes the shape a second backend plugs into, so that adding one is bounded work
against a stable interface rather than a fork of the pipeline.

The grounding is recorded in the learning
[the implementation design directions fold into one architecture — records everywhere is the foundation built first](../learnings/2026-07-10-the-implementation-design-directions-fold-into-the-architecture-records-everywhere-first.md),
whose survey of the design directions found the backend seam already latent in the pipeline. It does not
restate the component ABI ([component-abi.md](../contracts/component-abi.md)), the value-heap runtime
([value-heap-runtime.md](./value-heap-runtime.md)), or the build-tool interface
([build-tool-interface.md](../contracts/build-tool-interface.md)); it fixes the compiler-internal seam those
contracts sit beyond.

## The Pipeline Is Target-Neutral Up To A Single Seam

There is one point in the pipeline before which nothing depends on the output target and after which
everything does. Above it sit the passes that establish *what the program means* — resolution, inference,
compile-time evaluation, the poison and erasure checks — and the computation of the program's *boundary
layout*: which definitions are exported, under what names, with what solved parameter and result types, and
which definitions are reachable. Below it sits the translation of the meaning into a particular target. A
backend is a function of the meaning and the layout; it is chosen at the seam, and the choice is the
toolchain's, carried as an ordinary build input rather than baked into the compiler.

### A Backend Is A Function Of The Typed Core And A Target-Neutral Layout

A backend MUST be a function of the typed core representation and a target-neutral boundary layout, so that
selecting a backend is a branch at one seam and every pass above the seam is shared by every backend
unchanged.

The passes that establish a program's meaning — name resolution, type determination, compile-time evaluation,
the collection of compile-provable traps, and the erasure fence — MUST run above the seam and be independent
of the chosen backend, so that a program means the same thing whichever target it is emitted to and a
disagreement between two targets on a well-formed program is impossible above the seam
([reference-compiler.md §Types Are Solved Once And Read Downstream](./reference-compiler.md),
[§Compile-Time Evaluation Is One Reduction Tier](./reference-compiler.md)).

### The Boundary Layout Is Computed Once, Target-Neutrally, And Reused

The program's boundary layout — its exported entries by declared name, each with its solved parameter and
result types, and the set of definitions reachable from an export — MUST be computed once, above the seam,
and consumed by whichever backend is selected, so that the boundary a program presents is a target-neutral
fact and dead-code elimination is performed once rather than per backend
([reference-compiler.md §The Component Boundary Is Explicit Data](./reference-compiler.md)).

An index, an address, or an envelope structure specific to one target MUST be computed by that target's
backend rather than mixed into the shared layout, so that the shared layout carries only what every backend
needs and a target-specific concern lives in the target that has it.

### The Emitted Artifact Is Self-Describing By Kind

A backend MUST produce an artifact tagged with the kind of output it is, so that a consumer distinguishes a
component from source in another language by the artifact's declared kind rather than by inspecting its
bytes, consistent with the artifacts-in, artifacts-out boundary
([build-tool-interface.md](../contracts/build-tool-interface.md)).

## The Flat Instruction Rung Is A Property Of A Linearizing Backend

The flat instruction rung is not a shared stage every path descends through; it is what a backend produces
when its target is itself a linear instruction stream. A backend whose target has structured control flow
consumes the typed structured core directly — printing the core's conditionals, matches, bindings, and calls
as the target's own — and never constructs the flat rung. This refines the tree-above/flat-below division of
[intermediate-representations.md](./intermediate-representations.md): the flat sequence below the core is a
*backend's* representation of the core, produced by the backend that needs it, not a rung the core is
obligated to pass through before any backend sees it.

### A Backend Linearizes The Core Only If Its Target Is Linear

The flattening of the core into a flat instruction rung MUST be performed by a backend whose target is a
linear instruction stream, and a backend whose target has structured control flow MUST consume the typed
structured core directly rather than a flattened rung, so that flattening is a cost paid only by the backend
that needs it and a structured target is emitted from the structure the core already carries.

### Control Flow A Linear Target Cannot Express Is That Backend's Concern

A construct a particular target cannot express directly — a loop the flat instruction rung cannot hold, a
continuation the target has no primitive for — MUST be resolved within the backend for that target, by a
fixed helper or an equivalent the target does provide, rather than by a stage above the seam, so that a
limitation of one target does not constrain the core or another backend
([reference-compiler.md §Control Flow The Flat Rung Cannot Express Is A Fixed Helper](./reference-compiler.md)).

A capability one backend must decline because its target cannot yet express a construct MUST be an honest
decline of that backend rather than a decline of the compiler, so that a target-specific gap is attributed
to the target and another backend that can express the construct is not held back by it
([reference-compiler.md §A "No" Is A First-Class Value Produced Where The Decision Is Made](./reference-compiler.md)).

## A Backend Chooses A Value Strategy And States Its Consequences

A backend must decide how a compound value is represented in its target: as a handle into the shared
value-heap runtime, or as the target's own native aggregate. This is a backend's choice, not the language's,
because it is unobservable to a pure value-to-value computation — the observable input and output are the
same either way. Where it *is* observable — the sharing and persistence of a collection held across many
versions — the backend that chooses native aggregates must say so, because that is where the choice stops
being invisible.

### A Compound Value's Representation Is The Backend's Choice, Bounded By Observability

A backend MAY represent a compound value either as a handle into the shared value-heap runtime or as its
target's native aggregate, provided the choice is unobservable to a value-to-value computation — equal inputs
producing equal outputs — so that the representation strategy is a backend concern and a program's meaning
does not depend on it ([value-heap-runtime.md §A Collection's Representation Is Not Observable](./value-heap-runtime.md)).

A backend whose value strategy does not preserve a property the shared runtime guarantees — the cheap
persistence of a collection shared across many live versions — MUST record that the property is not preserved
where the property could be relied upon, so that a consumer that depends on cheap many-version sharing is told
the strategy does not provide it rather than discovering it as a performance cliff.

### A Backend Linking The Shared Runtime Preserves Its Semantics Exactly

A backend that represents compound values as handles into the shared value-heap runtime MUST obtain those
values through the runtime's operations under the same consume-and-borrow contract the component target uses,
so that linking the runtime yields the same observable behavior as the component target rather than a second
implementation of the value semantics
([value-heap-runtime.md §Constructors Consume And Accessors Borrow](./value-heap-runtime.md)).

## What A Second Backend Does Not Change

A backend is a rendering of a meaning already fixed above the seam. It therefore inherits, rather than
redecides, the decline boundaries the front establishes. This section records that a second backend widens
capability only where its target genuinely expresses more, and never narrows correctness.

### A Backend Inherits The Front's Decline Boundaries And May Only Widen Them By Target Capability

A construct the front declines for every target — a value form the type system does not yet solve, a
continuation shape the effect classifier refers to the general case — MUST remain declined under every
backend, and a backend MAY lift a decline only where its target directly expresses a construct another target
must emit a helper for, so that a backend never accepts a program the front's meaning has not sanctioned and
the difference between backends is confined to how much of the sanctioned meaning each target can express.

The meaning against which every backend's output is judged MUST be the one executable semantics, so that two
backends emitting the same program are two renderings judged against one oracle rather than two definitions of
behavior ([constitution §XIV](../../constitution.md);
[reference-compiler.md §Convergence Is Judged By Running The Artifact](./reference-compiler.md)).
