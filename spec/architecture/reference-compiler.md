# Reference Compiler Architecture

> **NORMATIVE — REFERENCE COMPILER ARCHITECTURE.** This document prescribes the internal architecture of
> a compiler built to the Cadenza *reference architecture*: the pipeline shape, the intermediate
> representations, and the disciplines that keep a from-scratch implementation correct and reproducible.
> Its RFC-2119 requirements bind a compiler built to this reference architecture; they are citable by the
> requirement gate for such a compiler.
>
> **Conformance to the language is defined by the two gates** — the requirement gate and the behavior gate
> ([compiler-pipeline.md](../capabilities/compiler-pipeline.md), [constitution §XIV](../../constitution.md)).
> A compiler that passes both gates by other means still conforms; this document does not narrow what a
> conforming compiler is. It exists because the gate obligations admit many pipeline shapes, and earlier
> generations repeatedly rediscovered — through the failure modes recorded in [learnings](../learnings/) —
> that most of those shapes do not survive contact with the whole language. This is the reproduction path:
> the architecture a fresh implementation follows to satisfy the capability specifications *without*
> re-deriving those failures.
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying exactly
> one obligation, under a stable heading. Per [constitution §XIII](../../constitution.md), the requirements
> below name no prior prototype, engine, or source path; the descriptive lead-ins and the learnings they
> cite carry the concrete grounding.

## Purpose And Scope

This document fixes *how* the reference compiler is built, where the capability specifications fix *what*
a compiled program means. It states five architectural disciplines and their obligations:

1. **The pipeline is a nanopass ladder** of typed intermediate representations, each pass a total function
   from one rung to the next, and the core representation is in A-normal form so value flow is explicit.
2. **Types are solved once and read downstream** — inference materializes a typed representation, and no
   later pass re-derives a type.
3. **Compile-time evaluation is a single reduction tier** — one evaluator folds constants, reduces
   generics, and makes definitions vanish, with a compile-provable trap failing the build.
4. **Every construct is an ordinary value reached by the ordinary mechanism** — nothing is privileged by
   name, and the resolver recognizes values, not names.
5. **The component boundary is explicit data** — the emitted interface is read from declared signatures,
   never inferred from a program's body.
6. **Instruction selection emits against a fixed runtime and envelope** — a flat instruction rung, fixed
   compiler-emitted helpers for control flow it cannot express, scratch-locals-or-decline for guarded ops,
   and a fixed component envelope derived from the runtime contract.
7. **Effects are classified first and resolved by monomorphization** — the common resumption shape lowers
   to plain code, and the discharging handler is a compile-time constant, not a runtime search.
8. **A "no" is a first-class value produced where the decision is made** — reject, decline, and trap are
   distinguished at the point of decision, ordered by safety, and never reconstructed downstream from a
   leaking proxy.

It does not restate the language semantics, the component ABI, or the byte-level contracts; those remain in
[capabilities](../capabilities/) and [contracts](../contracts/). The value-heap runtime the compiler emits
against — the artifact these programs import — has its own reference architecture in
[value-heap-runtime.md](./value-heap-runtime.md). This document realizes [overview §7](../overview.md),
[overview §8](../overview.md), [overview §10](../overview.md), and [overview §14](../overview.md), and it is
the architecture-level response to the failures recorded in [overview §16](../overview.md).

The grounding for every discipline below is a learning that records the failure it prevents:
[the nanopass ladder](../learnings/2026-07-09-the-compiler-is-a-nanopass-ladder-each-pass-an-exhaustive-match.md),
[solve-once/read-downstream](../learnings/2026-07-09-solve-the-type-once-read-it-downstream-never-re-derive.md),
[the one fold tier](../learnings/2026-07-09-const-folding-is-the-one-tier-poison-plus-dce-give-reachability.md),
[nothing privileged by name](../learnings/2026-07-09-everything-is-a-record-nothing-built-in-is-privileged-by-name.md),
and [the boundary is the signature](../learnings/2026-07-09-the-abi-is-the-signature-emit-names-verbatim-never-sniff-the-output-shape.md).

## The Nanopass Ladder

The reference compiler proceeds through a ladder of intermediate representations, one pass per rung: from
the abstract syntax tree, to a **resolved representation** in which names denote their bindings, to a
**typed representation** in which every node carries its solved type, to a **core representation** over
which compile-time evaluation runs, to an **instruction representation** serialized to bytes. These are the
rungs conventionally called AST, HIR, typed-HIR, MIR, and LIR; the requirements below name them by role.
This realizes and extends [compiler-pipeline.md §Emission Serializes A Lowered Representation](../capabilities/compiler-pipeline.md),
which fixes that a resolved analyzed middle exists without fixing its decomposition. Authoring the whole
compiler showed the decomposition is not optional guidance: the predecessor's single fused emit-walk could
not be repaired *because* it had no rung at which a single concern was the only concern, so every fix
perturbed every other.

### The Pipeline Is A Ladder Of Typed Intermediate Representations

The compiler MUST proceed through a sequence of intermediate representations in which each representation is
a value of a distinct type and each pass is a function from one representation to the next, so that a
concern has exactly one rung at which it is the only concern and cannot leak into the others.

### Each Rung Is A Typed Sum Matched Exhaustively

Each intermediate representation MUST be a sum type whose variants are the constructs it can hold, so that
the representation a pass consumes is a closed, inspectable set of cases rather than an open-ended value.

A pass over an intermediate representation MUST match that representation's variant set exhaustively, so
that a variant the pass does not handle is a compile-time error in the compiler itself rather than a silent
fall-through that miscompiles the construct.

### One Pass Owns One Concern

Name resolution MUST be performed once, at the pass that produces the resolved representation, so that no
later pass searches a scope to discover what a name denotes.

Type determination MUST be performed once, at the pass that produces the typed representation, so that no
later pass re-derives a type (§Types Are Solved Once And Read Downstream).

The step that emits instruction bytes MUST consume the instruction representation alone and MUST NOT
resolve a name, determine a type, choose an effect's handler, or fold a constant, so that emission is the
serialization of decisions already made ([compiler-pipeline.md §Emission Serializes A Lowered Representation](../capabilities/compiler-pipeline.md)).

### A Decision Is Carried Concretely, Never Re-Derived Downstream

A decision a pass fixes — the binding a name denotes, the slot a field occupies, the absolute index a call
targets — MUST be carried in the representation the pass produces as a concrete resolved value, so that a
later pass reads the decision rather than re-computing it.

The instruction representation MUST carry each call as its resolved target rather than as a name to be
looked up when bytes are laid, so that byte emission performs no relocation or index remapping and cannot
disagree with the pass that fixed the target.

### The Core Representation Is In A-Normal Form

The core representation over which compile-time evaluation runs MUST be in A-normal form — every
non-trivial subexpression bound to a name, and every operand of an operation a name or a constant — so that
the flow of every intermediate value is explicit rather than implicit in the nesting of an expression tree.

A pass that must know where a value is last used, or which values are live at a program point, MUST read
that from the named bindings of the A-normal form rather than reconstruct it from a nested tree, so that a
precise-reclamation analysis and a continuation-capture analysis consume an explicit value flow rather than
approximate one conservatively (§In-Place Reuse under [value-heap-runtime.md](./value-heap-runtime.md);
§Effects Are Classified First And Resolved By Monomorphization).

The one compile-time evaluation tier MUST eliminate the administrative bindings A-normalization introduces —
copy-propagating a binding used once and dropping a binding used never — so that naming every intermediate
adds no runtime cost and A-normal form is a normalization of the compiler's representation rather than of the
emitted component (§Compile-Time Evaluation Is One Reduction Tier).

## Types Are Solved Once And Read Downstream

Inference is a distinct pass that consumes the resolved representation and produces the typed representation
by solving type variables under unification ([type-system.md §Inference](../capabilities/type-system.md)).
Every rung below it *reads* the solved type; none re-derives one. This is the architectural fix for the
failure recorded in
[the coarse-kind post-mortem](../learnings/2026-07-08-a-coarse-kind-classifier-re-derived-at-emit-is-the-wrong-inference-and-fails-one-way-at-every-lattice-point.md):
a compiler that carried a coarse type classifier *re-derived during emission*, alongside a separate notion
of structure the two could disagree, produced a whole family of miscompiles — each an instance of one fault
— that closed only when inference was made a real, order-independent solve whose result is materialized and
read.

### Inference Materializes The Type Before Lowering

The compiler MUST solve every expression's type in the pass that produces the typed representation, before
any pass that lowers toward instructions, so that a lowering pass reads a determined type rather than
computing one as a side effect of lowering.

The typed representation MUST associate each node with its solved type, so that the machine representation a
value takes is obtained by reading its solved type rather than by re-inferring it.

### The Machine Representation Is A Read-Off Of The Solved Type

A pass that must choose the machine representation of a value MUST derive that choice from the value's
already-solved type, so that the compiler holds one notion of a value's type and no second notion can drift
from it.

### An Undetermined Type Is A Rejection, Not A Default

An expression whose type inference leaves undetermined MUST cause the compiler to decline or reject rather
than assume a default type, so that a value of unknown type is never emitted under a representation chosen
for convenience.

An inference solution MUST be independent of the order in which the compiler visits definitions and uses,
so that a self-reference, a forward reference, and a mutual recursion are typed by the same solution
regardless of traversal order ([type-system.md §Inference Is Principal-Type Inference By Unification](../capabilities/type-system.md)).

## Compile-Time Evaluation Is One Reduction Tier

A single evaluator reduces the core representation to itself, and it is the one tier
[metaprogramming.md §Compile-Time Evaluation Is One Tier](../capabilities/metaprogramming.md) requires:
the same mechanism folds constants, reduces generic instantiations, expands macros, and specializes
definitions applied to compile-time-known arguments. It is built as a general reduction of the language,
not an arithmetic peephole, which is what lets the constructs a naive compiler special-cases — a module, a
function value, a built-in operation, a constructor — reduce away through it rather than each needing
bespoke machinery.

### One Evaluator Performs Every Compile-Time Reduction

Constant folding, generic reduction, monomorphization, and the specialization of a definition applied to
compile-time-known arguments MUST be performed by one evaluator over the core representation, so that there
is one place the meaning of compile-time computation lives and the reductions cannot drift apart.

A construct whose value is fully determined at compile time — a module's export record, a function value
applied to its arguments, a built-in operation applied to constants, a constructor applied to its payload —
MUST reduce to its value through that evaluator and leave no trace of itself in the emitted component, so
that these constructs are compile-time structure rather than runtime cost.

### A Compile-Provable Trap Fails The Build

An operation all of whose operands the evaluator determines at compile time, and whose defined outcome on
those operands is a trap, MUST reduce to a poisoned term that carries the corresponding machine-readable
diagnostic, so that a provable failure is represented as a value the evaluator can propagate rather than as
emitted code.

A poisoned term that survives reduction to a position the program unconditionally reaches MUST fail
compilation with its diagnostic, so that a trap the compiler can prove is a compile-time rejection rather
than a component that traps at run time
([numeric-model.md §A Constant Operation With No Value Is Rejected At Compile Time](../capabilities/numeric-model.md)).

The compiler MUST report every reached poisoned term rather than stop at the first, consistent with
[compiler-pipeline.md §Phases Recover From Errors](../capabilities/compiler-pipeline.md).

### Reachability Is A Consequence Of Reduction

Reducing a conditional whose condition the evaluator determines MUST retain only the selected branch and
discard the unselected branch together with any poisoned term within it, so that a provable trap shielded by
a branch the program never takes stays a runtime trap rather than failing the build.

The collection of reached poisoned terms MUST descend into the positions a value is unconditionally used —
an operand, a product element, a call argument, a binding's value and body, a payload, a scrutinee — and
MUST NOT descend into a conditional's branches or a match's arm bodies, so that reachability is read from
the structure the evaluator already produced rather than computed by a separate analysis.

A definition unreachable from any export MUST NOT be emitted, so that dead code is eliminated as a
consequence of reachability rather than carried into the component.

### A Meaning-Preserving Rewrite Preserves Value And Checks

A rewrite that eliminates a subterm MUST preserve every check that subterm would have triggered — its type
agreement, its scope resolution, and its provable traps — and MAY eliminate only the subterm's evaluation,
so that a rewrite that is value-preserving does not become rejection-preserving-in-name-only by silently
accepting a program the eliminated subterm would have made ill-formed.

A branch or arm the evaluator proves is not taken MUST still be type-checked and scope-checked, so that an
unevaluated subterm cannot carry a deferred error, exactly as
[core-semantics.md §Conditionals Evaluate One Branch](../capabilities/core-semantics.md) requires whether or
not a fold eliminates it. This is the rule whose violation is recorded in
[a fold that eliminates a branch must not eliminate its type-check](../learnings/2026-07-07-a-fold-that-eliminates-a-branch-must-not-eliminate-its-type-check.md).

## Nothing Is Privileged By Name

The resolver recognizes *values*, not names. Every construct that a naive compiler would recognize by
spelling — a built-in module, a built-in operation, a type, a constructor, a pattern — is instead an
ordinary value reached by the ordinary lookup-and-project mechanism, so that one lookup rule and one
projection rule subsume what would otherwise be a special case per recognized name. This realizes
[core-semantics.md §A Built-In Module Is A Record Of Its Operations](../capabilities/core-semantics.md) and
[§Member Access Projects A Record Field](../capabilities/core-semantics.md), and generalizes the
[one-accessor learning](../learnings/2026-07-03-one-accessor-modules-are-records.md) from modules to the
whole compiler: every name heuristic the predecessor carried was a second code path that had to agree with
a first, which is the same disagree-and-miscompile class the ladder and solve-once disciplines remove at
other layers.

### The Prelude Is A Single Map The Resolver Consults By Name Alone

The language's built-in bindings MUST be presented to the resolver as a single collection of named values,
so that resolving a name is one lookup — the lexical scope, then that collection — with no case that
recognizes a particular built-in name in a particular position.

A program binding MUST be able to shadow a built-in binding of the same name under the ordinary shadowing
rule, so that a built-in name holds no privilege a program-defined name lacks
([core-semantics.md §A Built-In Module Is A Record Of Its Operations](../capabilities/core-semantics.md)).

### A Built-In Operation Is A First-Class Value, Lowered At Selection

A built-in operation MUST be represented as a first-class value carried unchanged through the resolved,
typed, and core representations, and MUST be translated to instructions only at instruction selection, so
that a built-in operation flows through the pipeline as a value rather than as a construct the earlier
passes special-case.

### A Type, A Constructor, And A Pattern Are Ordinary Values And Expressions

A type MUST be an ordinary value carried and reduced by the same machinery as any other value, so that a
generic instantiation is an ordinary application the one evaluator reduces rather than a distinct construct
([type-system.md §Generics Are Type-Valued Parameters](../capabilities/type-system.md)).

A pattern MUST be represented as an ordinary expression of the representation it appears in, distinguished
only by a binder leaf and a wildcard leaf, so that a pattern is resolved and lowered by the same passes as
the equivalent constructing expression rather than by a separate pattern construct.

### An Unrealized Built-In Field Declines Where A Missing User Field Rejects

A built-in module MUST be treated as open: a field naming an operation the compiler does not yet realize
MUST cause the compiler to decline rather than reject, so that an unimplemented built-in operation is a
capability the compiler lacks rather than a program that is ill-formed.

A user-defined record MUST be treated as closed: access to a field the record's type does not contain MUST
be rejected with the machine-readable code for an absent field, so that the open treatment of a built-in
module never widens to a program's own records
([core-semantics.md §Member Access Projects A Record Field](../capabilities/core-semantics.md)).

## The Component Boundary Is Explicit Data

The emitted interface is read from what the source declares, never inferred from what a function's body
does. This realizes [compiler-pipeline.md §Emission Serializes A Lowered Representation](../capabilities/compiler-pipeline.md)
and [modules-and-namespaces.md §Visibility](../capabilities/modules-and-namespaces.md) at the boundary, and
records as a discipline the failure the predecessor embodied: it walked an entry's body to guess the output
shape and renamed the entry to a blessed name, so the emitted bytes depended on a convention the source
never wrote and could disagree with what the body returned.

### The Exported Interface Is The Declared Signature

The interface a compiler emits for an export MUST be read from that export's declared signature — its
parameter types and its result type — rather than inferred by inspecting the export's body, so that the
boundary is a contract the source states rather than a shape the compiler guesses.

The compiler MUST NOT classify exports into privileged kinds distinguished by anything other than their
signatures, so that no export is recognized by name or by the shape of its body where the signature already
carries the distinction.

### An Export Name Crosses The Boundary Verbatim

The compiler MUST emit an export under the name the source declares for it, without renaming, so that the
emitted interface is predictable from the source and a consumer resolves an entry by its signature rather
than by a name the compiler assigned ([modules-and-namespaces.md §Visibility](../capabilities/modules-and-namespaces.md)).

### The Encoding Belongs To The Serializer Alone

Every pass above the step that emits bytes MUST reason in named representation types rather than in the
target format's encoding, so that the raw encoding of an instruction or a value type is written in exactly
one place and no analysis pass hard-codes an encoding byte.

## Instruction Selection Emits Against A Fixed Runtime And Envelope

Instruction selection turns the core representation into a flat instruction sequence, and the serializer
lays that sequence into a component envelope. This is where a from-scratch implementation most often
reaches for a shortcut that later miscompiles: open-coding control flow the flat rung cannot express,
sharing scratch machinery that a stricter byte-identity target then penalizes, or hand-writing an
encoding that sign-extends a constant. The disciplines below record the shapes that hold. A program that
touches no compound value emits against no runtime import; a program that constructs or inspects a
compound value emits against the fixed value-heap runtime ([value-heap-runtime.md](./value-heap-runtime.md)).

### A Value's Machine Representation Follows Its Solved Type At Selection

Selection MUST choose whether to box a scalar into a heap handle or leave a compound value as the handle it
already is by reading the value's solved type, so that the boxing decision is a read-off of the type rather
than a guess from the instruction node's shape (§The Machine Representation Is A Read-Off Of The Solved Type).

A call MUST be selected to the callee's absolute position in the emitted function space, fixed once by the
layout, so that byte emission relocates no call and cannot disagree with the layout about a callee's index
(§A Decision Is Carried Concretely, Never Re-Derived Downstream).

### A Consuming Operation Retains A Shared Operand Before Consuming It

Selection MUST retain a reference to an operand that a consuming runtime operation would consume when that
operand may be used again, so that the runtime derives a new value rather than mutating a value another
reference can observe, consistent with the runtime's consume/borrow contract
([value-heap-runtime.md §Constructors Consume And Accessors Borrow](./value-heap-runtime.md)).

A retention that is not needed MUST be safe to emit, and an under-retention MUST NOT occur, so that the
conservative choice — retaining a last-use operand — costs at worst an unobservable missed reuse while
never corrupting a shared value.

### Control Flow The Flat Rung Cannot Express Is A Fixed Helper, Not A New Instruction

Control flow that the flat instruction rung cannot express — a loop over a value's bytes or elements — MUST
be emitted as a fixed compiler-emitted helper the runtime component carries, called by a plain call, rather
than by extending the instruction rung with structured-control-flow variants, so that instruction selection
emits only a call and a two-way branch and the flat rung stays flat.

The set of fixed helpers MUST be a closed, program-independent part of the runtime component, so that a
helper is shared machinery generated once rather than open-coded per program.

### A Guarded Operation Reserves Bounded Scratch Or Declines

An operation whose correct emission needs a trap guard MUST reserve the scratch locals that guard needs,
allocated past the function's parameters and bindings, and MUST reserve no more than it needs, so that a
guard is emitted inline as a bounded sequence and a shared guard mechanism does not tax a client that needs
less (the byte-identity cost of over-reserving is recorded in
[the shared-scratch-local learning](../learnings/2026-07-07-sharing-the-scratch-local-mechanism-cost-right-shift-its-byte-identity.md)).

An operation whose correct emission would need control flow the flat rung and a bounded scratch set cannot
express MUST decline rather than emit a plausible sequence, so that a construct awaiting a later phase is a
clean decline rather than a wrong result (§A "No" Is A First-Class Value Produced Where The Decision Is Made).

### The Component Envelope Is Derived From The Runtime Contract

The component envelope that wraps a program's emitted core — its imports, its instantiation, and the
lifting of its exports — MUST be a fixed function of the runtime interface the program imports, generated
from that interface's declaration rather than hand-maintained, so that adding a runtime operation
re-derives the envelope rather than requiring a hand-edit that can drift.

An index or count the envelope depends on — the number of runtime imports, the base position of the first
program-defined function — MUST be derived from the runtime interface rather than written as a literal, so
that the emitted indices cannot drift from the interface as it grows.

### Emission Is Validated Byte-Identical To An Independent Encoder

The bytes the compiler emits MUST be validated byte-identical to those an independent reference encoder
produces for the same input at a covering set of small cases, so that hand-emitting the envelope — which is
what lets the emitter carry no external encoder in its own byte path — is licensed by an oracle rather than
trusted.

A signed constant MUST be emitted through the signed variable-length encoding the target format defines,
never as a raw byte, so that a constant whose low byte has its high bit set is not silently sign-extended to
a different value.

## Effects Are Classified First And Resolved By Monomorphization

The semantics of effects and handlers are fixed in [capabilities-and-effects.md](../capabilities/capabilities-and-effects.md),
which already requires that the discharging handler be determined statically by monomorphizing the handler
context over the closed effect row (§Handler Resolution Is Dynamic In Extent And Statically Determined) and
that a continuation be one-shot by default (§A Continuation Is One-Shot By Default). This section fixes the
*compilation strategy* that realizes those requirements: classify each handler arm and lower each class to
the cheapest mechanism it needs, so the common case carries no continuation machinery at all. It is grounded
in [compiling effect handlers, classify first](../learnings/2026-07-06-compiling-effect-handlers-classify-first-tail-resumptive-is-plain-code.md)
and [inlining resolves cross-function until recursion](../learnings/2026-07-06-implementing-effects-in-the-seed-inlining-resolves-cross-function-until-recursion.md).

### Each Handler Arm Is Classified By Its Resumption Shape

The compiler MUST classify each handler arm by how its body uses the resumption — one class that resumes
exactly once in tail position, one class that never resumes, and one class that resumes non-tail or captures
the continuation — and MUST lower each class to a distinct mechanism, so that the resumption shape a program
actually uses determines the machinery it pays for.

An arm's class MUST be the least upper bound over all of its control paths, so that a runtime branch inside
an arm never changes the reification decision, which is fixed at the point the operation is performed.

The compiler MUST treat an arm it cannot prove resumes at-most-once-in-tail or never as the general
resuming class, so that a misclassification never silently drops the work a non-tail resumption performs
after it resumes — the conservative direction, because the alternative is a miscompile.

### A Tail-Resumptive Arm Lowers To Plain Code

An arm that resumes exactly once in tail position MUST be lowered without reifying a continuation — by
emitting the arm body with the tail resumption replaced by its value and the handler's threaded state
carried forward — so that the common case a program's handlers use carries no continuation object, no
runtime handler stack, and no evidence structure.

### The Discharging Handler Is A Compile-Time Constant

The compiler MUST resolve the handler that discharges each performed operation to a compile-time constant,
so that no runtime handler search occurs, realizing
[capabilities-and-effects.md §Handler Resolution Is Dynamic In Extent And Statically Determined](../capabilities/capabilities-and-effects.md).

The compiler MUST resolve a cross-function performance by making the caller's handler present in the
callee — inlining a non-recursive callee, and specializing a recursive one once per handler context it is
called under — so that handler resolution is the one compile-time reduction tier applied to the handler
context, not a second specialization mechanism (§Compile-Time Evaluation Is One Reduction Tier).

A specialized recursive function MUST carry each enclosing handler's threaded state through its own call
boundary rather than through shared mutable storage, so that a nested or re-entrant handler context does not
clobber another's state.

An unbounded handler context — a recursion that installs a fresh handler per call — MUST cause the compiler
to decline rather than specialize without bound, and the bound MUST hold for the smallest target the
compiler runs on, so that an unbounded specialization is a clean decline rather than a crash of the
compiler.

### An Arm Body's Own Performances Resolve At Its Definition

A performance within a handler arm's body MUST resolve against the handlers enclosing the arm's definition,
not those enclosing the point the discharged operation was performed, so that a forwarding or interposing
handler re-performs into the context it was written in
([capabilities-and-effects.md §A Handler May Interpose](../capabilities/capabilities-and-effects.md)).

### A Reified Continuation Does Not Span A Host Call

A reified intra-program continuation MUST NOT span a host call, so that a host that re-derives a run from
its recorded responses never has to reconstruct a chain of run-local handles that are not canonical data —
keeping the two continuation notions distinct, the host-bound one canonical data and the intra-program one a
run-local structure ([durable execution is effects plus determinism](../learnings/2026-07-04-durable-execution-is-effects-plus-determinism.md)).

The intra-program effect layer MUST NOT alter the emitted component's import surface, so that handling an
effect in-program adds nothing to the manifest and a reified continuation is built from the existing runtime
value operations rather than a new import.

## A "No" Is A First-Class Value Produced Where The Decision Is Made

A compiler under construction compiles a strict sublanguage, and how it says "no" is as load-bearing as how
it says "yes." The three kinds of "no" — a rejection (the program is ill-formed), a decline (the compiler
does not yet handle the construct), and a trap (a runtime halt) — are already required to be machine-
branchable ([diagnostics.md §A Diagnostic Names Its Kind](../capabilities/diagnostics.md)), and a construct
the generation cannot compile is already required to decline rather than miscompile
([self-hosting-and-bootstrap.md §An Unsupported Construct Is Declined, Not Miscompiled](../capabilities/self-hosting-and-bootstrap.md)).
This section fixes the *internal discipline* that keeps those guarantees true as the compiler is built:
where the "no" is decided, how its kinds are ordered, and why a proxy that reconstructs it downstream leaks.
It is grounded in [decline, do not miscompile](../learnings/2026-07-03-decline-do-not-miscompile.md),
[a type check has two opposite failure modes](../learnings/2026-07-07-a-type-check-has-two-opposite-failure-modes-and-over-rejecting-valid-code-is-the-worse-one.md),
and [a decline that leaks into a valid-but-trapping component](../learnings/2026-07-07-a-decline-that-leaks-into-a-valid-but-trapping-component-is-the-most-dangerous-shape.md).

### Outcomes Are Ordered By Safety

The compiler MUST treat a wrong value and a component that traps where the source denotes no trap as
stricter violations than a clean decline, and a clean decline as a stricter outcome than a correct
compilation, so that a construct the compiler cannot yet compile correctly declines rather than emits a
wrong value or a valid-but-trapping component.

When a partial fix regresses, the compiler MUST be moved toward the safer outcome — a decline — rather than
back toward a wrong value, so that a revert that restores a known-"working" state does not reintroduce a
miscompile that state contained.

### The Kind Of A "No" Is Fixed Where It Is Produced

The compiler MUST determine whether an outcome is a rejection, a decline, or a trap at the point the outcome
is produced and carry that classification as a distinct value, rather than reconstruct it downstream from
the emitted artifact's shape, so that a genuine rejection and an honest decline are never conflated by a
single sink they both flow into.

A decline MUST NOT be emitted as a valid component that traps at run time, so that a construct the compiler
declines is an observable decline rather than the most dangerous shape a decline can take — a valid-looking
component whose failure only running it reveals.

### A Conservative Check Is Silent On What It Cannot Prove

A check that cannot positively prove a construct ill-formed MUST decline to reject it, so that over-rejecting
a well-typed program — which denies a correct program its meaning — is avoided even at the cost of a missed
rejection, the less-harmful of the two failure directions.

A check MUST be silent on a construct whose kind it does not recognize rather than fail on it, so that
encountering an unmodeled construct is never a crash of the compiler.

## The Reader, Printer, And Renderer Are Built As Duals

The compiler's text and byte surfaces — the reader from bytes to the syntax tree, the printer back to text,
and the renderer of a runtime result — are required to exist and round-trip
([self-hosting-surface.md](../capabilities/self-hosting-surface.md)). This section fixes the *construction*
that makes them correct: the reader as the dual of the emission spine, name resolution that respects
shadowing, and a renderer that walks a static shape because the runtime holds no names. It is grounded in
[the reader decodes the input dual of the output spine](../learnings/2026-07-07-the-reader-decodes-cbor-as-the-input-dual-of-the-output-spine.md)
and [the runtime is tag-free — rendering walks a static shape](../learnings/2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape.md).

### The Reader Is The Input Dual Of The Emission Spine

The reader that decodes the canonical binary syntax tree MUST be built from the same small operation
vocabulary the emitter uses to produce bytes, so that decoding is the inverse of encoding over one shared
vocabulary rather than a second, independently-drifting byte surface.

A byte-decoding surface MUST be complete over its three legs — selecting an operation from a head index,
iterating a sequence by a decoded length, and interpreting each leaf by its encoded kind — so that
completeness of the decoder is checkable against those three legs rather than assumed.

### Name Resolution Searches The Innermost Binding First

Name resolution MUST search an ordered scope environment innermost-first, so that a name a nearer binding
shadows resolves to the shadowing binding rather than silently to the outer binding the shadow exists to
hide.

The compiler MUST distinguish a call from an operator by membership in the environment of function
bindings, so that call-versus-operator resolution and scope resolution are the same lookup over different
environments rather than a heuristic over a name's spelling.

### Rendering Walks A Static Shape And Supplies The Names

The compiler MUST render a runtime value by emitting code that walks the value's static type and supplies
the field and variant names from that type, because the runtime holds no names, so that rendering is
type-directed and the runtime stays name-free ([value-heap-runtime.md §The Heap Holds Structure And Data, Never A Type Or A Name](./value-heap-runtime.md)).

To render a value of a recursive type, the compiler MUST read the type's declaration and cut each self-
reference to a named back-reference rather than inline the type's constructor, so that rendering a recursive
value terminates over a finite shape graph rather than diverging on an infinite unrolling.

A render-and-parse pair MUST be checked by round-tripping through the inverse function rather than through
the renderer alone, so that a renderer that emits text its reader cannot read back is caught rather than
hidden by an oracle that launders both sides through the same renderer.

## Convergence Is Judged By Running The Artifact

Bringing the compiler to correctness is judged by a differential gate, and how that gate is read is itself a
discipline. The behavior gate already requires that a case be discharged by *executing* it, not by inspecting
the shape of the code ([conformance-gate.md §A Behavior Requirement Is Covered Only By Execution](../capabilities/conformance-gate.md)),
and an unsupported construct is required to decline observably-distinctly from a divergent one
([self-hosting-and-bootstrap.md](../capabilities/self-hosting-and-bootstrap.md)). This section fixes the
construction disciplines that keep the gate honest and the compiler's cost bounded. It is grounded in
[the byte gate absorbs the wrong-sweep as native classification](../learnings/2026-07-07-the-byte-gate-absorbed-the-loops-hand-run-wrong-sweep-as-native-classification.md)
and [settledness is the artifact ceasing to change](../learnings/2026-07-07-settledness-is-the-artifact-not-changing-not-the-metric-trending-good.md).

### The Gate Runs The Artifact And Discriminates A Decline From A Disagreement

A differential gate MUST classify a mismatch by running the compiled artifact rather than by inspecting its
shape, so that a decline stub is not counted as a disagreement and a runtime-trapping decline is not missed
by a syntactic proxy.

A gate MUST discriminate a decline from a disagreement in both mismatch directions — where the reference
accepts and the compiler differs, and where the reference rejects and the compiler accepts — so that a
one-sided discriminator does not silently over-count one half of the frontier as disagreement.

A differential gate MUST report agreement, byte-differing agreement, decline, and disagreement as separate
counts, so that zero disagreement is read as soundness rather than misread as completeness while the decline
count still measures the remaining coverage.

### Compilation Cost Is Bounded In Nesting Depth

The compiler MUST NOT let its compilation cost grow exponentially in the nesting depth of a program's
scoping and branching constructs, so that a deeply nested program compiles in time proportional to its size
rather than to an exponential of its depth — achieved by materializing and sharing an analysis result rather
than re-deriving it on each descent.

A fixpoint the compiler iterates MUST materialize its result rather than re-walk the structure it analyzes,
so that reaching a fixed point does not reintroduce the re-derivation cost the materialization removed.
