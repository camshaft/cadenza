# Effects lower by classify-first — the common resumption is plain code, and the handler is a compile-time constant

*2026-07-09*

**What happened.** The compilation strategy for effects and handlers settled on **classify-first**: a
single compile-time pass classifies each handler arm by how its body uses the resumption, and lowers each
class to the cheapest mechanism it actually needs.

- **Tail-resumptive** (resumes exactly once, in tail position) → **plain code, no continuation reified.**
  Emit the arm body with the tail resumption replaced by its value and the handler's threaded state carried
  forward. Every arm the corpus and the self-hosting compiler use is tail-resumptive, so the *entire
  shipping surface* compiles with zero continuation machinery — no continuation object, no runtime handler
  stack, no evidence vector.
- **Abortive** (never resumes) → early exit, no capture.
- **General one-shot** (resumes non-tail, or captures the continuation) → the machinery it genuinely needs:
  a reified continuation built from the existing runtime value operations.

An arm's class is the **least upper bound over all its control paths** — a runtime branch inside an arm
never changes the reification decision, which is fixed at the perform site. And the classifier is
**conservative**: anything not provably tail or abortive is general one-shot, because misclassifying a
non-tail arm as tail silently drops the work after the resume — a miscompile, the one thing the project
never does.

The discharging handler is a **compile-time constant**. "Dynamic in extent but statically determined" is
realized by resolving each performance to a concrete handler arm at compile time — no runtime handler
search. Cross-function performances resolve by making the caller's handler present in the callee:
**inlining** a non-recursive callee, and **specializing** a recursive one once per handler context it is
called under, threading each context's state through the call boundary rather than through shared mutable
storage. This is not a second specialization mechanism — it is the one compile-time reduction tier
([[2026-07-09-const-folding-is-the-one-tier-poison-plus-dce-give-reachability]]) keyed on handler context
alongside type monomorphization. An unbounded handler context (recursion installing a fresh handler per
call) declines rather than specializing without bound, and the decline bound must hold for the smallest
target the compiler runs on — a native-vs-wasm differential caught a guard that tripped only on the larger
host stack.

Two boundary rules keep durable execution sound. An arm body's own performances resolve against the
handlers enclosing the arm's **definition**, not the perform site (so a forwarding/interposing handler
re-performs into the context it was written in). And a **reified intra-program continuation must not span a
host call** — a host that re-derives a run from its recorded responses cannot reconstruct a chain of
run-local heap handles, so the two continuation notions must never share an object: a host-bound
continuation is canonical data (migratable), an intra-program one is a run-local structure re-derived by
replay.

**Why.** The naive implementation of algebraic effects reifies a continuation at every perform and carries
a runtime handler stack — and then pays for it on every effectful program, including the overwhelming
majority whose handlers only tail-resume. Classify-first inverts that: the cost follows the resumption
shape the program actually uses, and because the discharging handler is statically known, the common case
collapses to plain code with the state threaded as an ordinary value. This is the same "make the special
thing ride the general mechanism" move the rest of the compiler makes — handler resolution reduces away
through the one evaluator the way a generic instantiation or a module export record does, rather than
needing a bespoke effect-resolution engine. The conservatism is the reject-don't-miscompile discipline
([[2026-07-03-decline-do-not-miscompile]]) at the classifier: a tail misclassification is a silent
wrong-value, so the classifier must *prove* tail-ness, not assume it. The host-composition invariant is
what makes effects-plus-determinism equal durable execution
([[2026-07-04-durable-execution-is-effects-plus-determinism]]): tiers that reify nothing on the stack leave
a host free to re-derive across a host call, and the one configuration re-derivation could not
reconstruct — an ephemeral heap-handle continuation spanning a host call — is exactly the thing the
invariant forbids.

Reproduction note: the semantics were corrected (2026-07-06) from "resolved lexically" to "dynamic in
extent, statically determined by monomorphizing the handler context." A restart must use the corrected
framing throughout the cross-function/recursive cases; "lexical handler context" is stale wording in
exactly the places specialization does the work.

**The requirement it drove.** New normative section in
[reference-compiler.md §Effects Are Classified First And Resolved By Monomorphization](../architecture/reference-compiler.md):
each arm is classified by resumption shape (least-upper-bound over control paths, conservative toward the
resuming class); a tail-resumptive arm lowers to plain code; the discharging handler is a compile-time
constant resolved cross-function by inline-then-specialize on the one reduction tier, threading each
context's state through the call boundary, declining an unbounded context; an arm body's performances
resolve at its definition; a reified continuation does not span a host call and the intra-program layer adds
nothing to the import surface. Realizes and does not restate
[capabilities-and-effects.md §Handler Resolution Is Dynamic In Extent And Statically Determined](../capabilities/capabilities-and-effects.md)
and §A Continuation Is One-Shot By Default. The concrete classification predicates, the reified-frame
encoding, and the specialization keying are declared-default/internal. This is the live workstream (the
compiler's actual next gap is member access on a fresh type-variable), so the section is authored as the
design is built rather than reconstructed after.
