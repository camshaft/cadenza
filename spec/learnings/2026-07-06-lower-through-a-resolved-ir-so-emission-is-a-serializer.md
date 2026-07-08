# Lower through a resolved IR so emission is a serializer, not a construction site

*2026-07-06*

**What happened.** As the effect system grew, the compiler's single lowering pass —
`emit(node) -> (bytes, kind)`, one recursive walk over the homoiconic AST — was doing five jobs at
once: rejecting ill-typed forms, constant-folding, inlining lambdas by re-emitting them under a captured
environment, resolving which handler discharges a performed effect, and appending the wasm bytes. There
was no representation *between* the AST and the bytes; the bytes were the only artifact, produced while
every decision was still being made. Two symptoms made the cost concrete. First, handler resolution was
carried in a **mutable stack the emitter mutates as it runs**, so lowering a handler arm had to save,
truncate, and restore that stack by hand to make a nested same-effect perform resolve against the
handlers in scope *where the arm was written* rather than where it was performed — the effects how-to
flags this in bold as the one place a miscompile hides, "test nested same-effect handlers first." Second,
inlining by re-emitting a node under a cloned environment made the compiler **exponential in nesting
depth**, because resolution was redone, and the environment recloned, on every branch. Both are
properties of *doing analysis during emission*, not of the features themselves. This extends the earlier
finding that the **backend** IR should be a typed instruction sum
([[2026-07-05-the-internal-ir-is-a-typed-sum-the-public-ast-stays-homoiconic]]): that fixed the *last*
rung (bytes are serialized from a closed sum, exhaustively); the gap here is the *middle* rung — the
compiler had no resolved, analyzed representation to serialize *from*.

**Why.** Emission fused with analysis has no seam a new feature or optimization can be added at. A
control feature (abortive handlers, general one-shot resumption, a scheduler) or an optimization
(peephole, common-subexpression, better `match` compilation) has nowhere to live except *more branches
inside the byte emitter* — each one now also responsible for getting the bytes right, and each one able
to break the delicate interleaving the existing ones depend on. The under-frame trap is the general case
in miniature: when *which handler discharges an operation* is decided by state the emitter accumulates,
correctness depends on emission **order**, and order is the one thing a growing compiler cannot hold
still. The language already says the opposite is achievable — a performed operation's handler is
determined by the program's structure (dynamic in extent, **statically determined**;
[[2026-07-06-compiling-effect-handlers-classify-first-tail-resumptive-is-plain-code]],
[[2026-07-06-implementing-effects-in-the-seed-inlining-resolves-cross-function-until-recursion]]) — so
that decision *can* be made once, ahead of emission, as a property of a resolved representation rather
than of the emitter's running state. The general resolution: put a representation between the AST and the
instructions in which names are **resolved to their bindings** and each analysis — type-checking,
folding, effect lowering — is a **transformation of that representation**, leaving byte emission to do
nothing but serialize decisions already made. Then a control feature is a rewrite of that
representation that emits no bytes at all, and it cannot perturb emission because emission is downstream
of it. This is the same instinct the compiler already reaches for when a pass needs structure the AST
does not carry — the type-directed value shape the renderer walks is a small IR of exactly this kind —
generalized to the whole program.

The requirements this drove are **representational**, not behavioral: like the sibling §Representation
requirements they are discharged by the requirement gate (an implementation and a test citation), and
they are honest rather than a shape-check that passes without enforcing anything
([[2026-07-02-a-modeled-subsystem-passes-a-shape-check]]) because each has an observable consequence — a
resolved representation is what makes the effect resolution the corpus already pins (nested same-effect
handlers, deep-chain resolution, one function under two handlers) come out right regardless of the order
the emitter happens to visit nodes. They deliberately do **not** prescribe the number of intermediate
representations, their variants, or the pass list; they state the obligations from which a solid pipeline
is rediscovered, and leave the ladder to the implementation. The seed generation may still fuse these
concerns — it is a bootstrap under the same latitude that lets it defer static typing (Core Principle
VII) — while the requirements fix the shape the compiler takes as it is authored in the full language.

**The requirement it drove.** `spec/capabilities/compiler-pipeline.md` §Representation gained two
headings between the existing AST-input and instruction-sum requirements:

- §"The Compiler Resolves Names Before It Selects Instructions" — the compiler MUST lower the AST to an
  intermediate representation in which every name reference is resolved to the binding it denotes before
  it selects the instructions to emit (so selection reads a resolved binding rather than searching a
  scope), and MUST determine the handler that discharges each performed effect operation from the
  structure of that resolved representation (so the discharging handler is fixed before emission rather
  than by state the emitter accumulates as it runs).

- §"Emission Serializes A Lowered Representation" — the compiler MUST perform name resolution, type
  checking, and each transformation it applies to a program (such as constant folding or effect
  lowering) as a transformation of its intermediate representation rather than as an effect of emitting
  instruction bytes; and the step that emits instruction bytes MUST consume an already-lowered
  representation and MUST NOT itself resolve a name, decide a type, or choose an effect's handler, so
  that emission is the serialization of decisions already made.

Together with the standing §"The Compiler Operates On AST Values" requirements (AST in, a typed
instruction sum out, serialized by an exhaustive match), these name all three rungs — AST, a resolved
analyzed middle, and the instruction sum — without prescribing the passes between them, so the pipeline's
architecture is a requirement the gate can cite rather than a convention the code is trusted to hold.
