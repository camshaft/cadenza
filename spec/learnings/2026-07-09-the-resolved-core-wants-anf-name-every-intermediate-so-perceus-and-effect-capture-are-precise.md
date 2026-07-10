# The resolved core wants to be A-normal form — name every intermediate, so Perceus and effect-capture are precise rather than conservative

*2026-07-09*

**What happened.** A review of the native reference compiler `rcdzc`, prompted by external AI-native-language
research that kept surfacing A-normal form / flattening as a recurring design choice
([[2026-07-09-the-paren-problem-is-a-decoding-problem-and-the-ai-native-win-is-semantic-context-at-the-edit-point]],
finding 4), found that **`rcdzc` performs no A-normal-form normalization anywhere in its pipeline.** `lower.rs`
is a self-described "shape-preserving map": a nested `(f (g x) (h y))` lowers to a nested `Mir::Call` whose
arguments are recursively-lowered nested `Mir::Call`s, and the tree stays nested from the AST through `select`.
There is no let-insertion, no linearization, no naming of intermediate results. This is a clean absence, not a
defect — and the distinction matters, because an *earlier* generation (the pre-rewrite seed) did depend on
named bindings by convention and paid for it: `(let ((r (dec 4))) (tuple.0 r))` compiled while the un-bound
`(tuple.0 (dec 4))` emitted an *invalid component*, because shape recovery was wired to the `let`-binding site
rather than the projection operator ([[2026-07-07-runtime-tuple-projection-needs-a-let-and-the-direct-path-miscompiles]],
SPEC-BACKLOG 15). `rcdzc` does **not** have that bug — it solves types properly with real inference and reads
the solved type downstream ([[2026-07-09-solve-the-type-once-read-it-downstream-never-re-derive]]), so it never
needs the old seed's bind-to-recover-the-shape hack. So the finding for the current compiler is not "ANF done
badly"; it is "no ANF, and two things that would be *precise* under ANF are currently *conservative* without it."

The two things, both real and both in the current code:

1. **Perceus is conservative because the IR does not name intermediates.** `select.rs`'s `emit_consuming_operand`
   decides whether to `dup` a value before a consuming runtime operation (`vec-push`, `bytes-concat`, …) by a
   *syntactic* test: if the operand is a bare `Local` it "may be used again in the same scope (shared)," so it
   dups defensively; a non-`Local` (a freshly-built nested subexpression) is assumed unshared and is not duped.
   The code names its own ceiling verbatim: *"A conservative Perceus … it may `dup` a last-use local too (rc 2,
   a missed FBIP + a leak) … A precise per-scope linear-use analysis is the eventual Perceus; this is the safe
   subset."* The over-dup is safe (a missed reuse and a leak, never corruption — `drop` is currently a no-op),
   but it is imprecise *because a nested tree does not carry an explicit use-count per value*. In A-normal form
   every intermediate is a named binding with explicit use-sites, so "is this value used again" becomes a
   count, not a "bare `Local`? dup to be safe" heuristic — which is exactly the input a precise Perceus/FBIP
   drop-insertion pass consumes.

2. **Effect-capture has no explicit live set to capture.** The planned effects lowering
   ([[2026-07-09-effects-lower-by-classify-first-and-resolve-by-monomorphization]], the live workstream)
   reifies a general one-shot continuation as a frame of the live values captured at the point an operation is
   performed. A nested expression tree does not *name* those live values — they are anonymous positions on an
   evaluation stack — so a nested IR forces the continuation-reification pass to reconstruct the capture set
   that A-normal form would hand it directly, one named binding per live intermediate.

**Why.** A-normal form is the representation in which *every non-trivial subexpression is bound to a name and
every argument is a name or a constant* — so the program is a linear sequence of named bindings ending in a
tail. It is not a nesting-cleanup nicety; it is the substrate the compiler's hardest downstream passes are
implicitly asking for. Precise reference counting (Perceus) and its in-place-reuse optimization (FBIP) are
last-use analyses, and a last-use analysis wants each value named with its uses enumerated — which is the
definition of A-normal form. Continuation reification wants the live set at a program point named — same thing.
The compile-time evaluator's β-reduction and the poison/reachability walk
([[2026-07-09-const-folding-is-the-one-tier-poison-plus-dce-give-reachability]]) are simpler over a flat
sequence of bindings than over a nested tree. And the AI-native reading view the research prizes — a flattened,
linear rendering where every intermediate has a name — is the same shape at the projection layer. One
normalization serves all four, which is why the recurring appearance of ANF/flattening across independent
research is not a coincidence: it is the canonical answer to "make the value flow explicit," and every one of
those consumers needs the value flow explicit.

The reason to state this now, before it is built, is that the cost of ANF is real and *conditional on one
thing*: naive A-normalization inserts an administrative binding for every subexpression, including trivial ones,
bloating the IR and the emitted code with "administrative redexes." A-normal form is only free if the
compile-time evaluator copy-propagates and dead-let-eliminates those administrative bindings back out — and
`rcdzc` already has the harder half of that (cross-function β-reduction in the one fold tier), so the required
cleanup is an extension of an existing pass, not new machinery. The one genuine design decision is *where* ANF
sits relative to inference: normalizing **at lowering** (so the mid-level core `Mir` is A-normal, below
inference) is the low-risk placement — it does not touch let-generalization, since administrative bindings never
reach the generalizer, and every consumer that wants ANF (Perceus, effect-capture, the eval tier) lives below
inference anyway. Normalizing **at resolution** (so the high-level core is A-normal from the start) gives the
reading view and queryable oracle a named handle for every intermediate earlier, but requires marking
administrative bindings non-generalizing so inference does not over-generalize them. The reproduction-critical
point is that A-normal form is a deliberate *choice for the resolved core*, made once, with administrative-redex
elimination as its non-optional companion — not a transformation to reach for reactively at the one operator
that happens to need a named operand, which is the trap the old seed fell into.

**The requirement it drove.** No behavioral requirement — A-normal form changes the compiler's internal
representation, not the language's meaning; a value-correctness (behavior-gate) oracle is invariant under it,
while the byte-identity anchor shifts until administrative bindings are eliminated, so "agreement" post-ANF is
byte-identity *after* administrative-let elimination. This is candidate content for a new
[reference-compiler.md](../architecture/reference-compiler.md) requirement under §The Nanopass Ladder — that the
resolved core is in A-normal form (every non-trivial subexpression bound, every argument a name or constant), so
that value flow is explicit for the precise last-use analysis a reference-count-with-reuse discipline consumes
([value-heap-runtime.md §In-Place Reuse Fires Only At A Unique Reference](../architecture/value-heap-runtime.md))
and for the live-set capture a reified continuation needs
([reference-compiler.md §Effects Are Classified First](../architecture/reference-compiler.md)) — paired with a
companion requirement that the one compile-time evaluation tier eliminates administrative bindings so ANF adds no
runtime cost. The before-versus-after-inference placement is the open decision that requirement's exact wording
waits on; lowering-first is the recommended start. Until then it is recorded here and as a pipeline design
choice, not folded, so a gate-gating MUST is not written ahead of the pass that would bind it.
