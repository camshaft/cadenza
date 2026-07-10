# Solve the type once with real HM, then read it downstream — never re-derive a coarse type at emit

*2026-07-09*

**What happened.** `rcdzc`'s inference (`infer.rs`, over `ty.rs`) is real Hindley-Milner run as a separate
`Hir → typed-Hir` pass *before* lowering: fresh type variables (`TVarSupply::fresh`), a single
substitution threaded across the whole module (`Subst`, `Subst::apply` recursing into compound element
types), commutative `unify` with an occurs-check, and let-generalization. Two disciplines make it the fix
for the bug family the rebuild was undertaken to kill, rather than a re-spelling of it:

1. **Signatures before bodies.** Every function is assigned a signature of fresh vars *before any body is
   inferred*, so a self-call, a forward call, and mutual recursion all unify against the callee's signature
   vars regardless of definition order. Order-independence is then a *property of unification* (`unify(Var,
   Int)` and `unify(Int, Var)` both bind the var), not a tie-break table or a first-write-wins slot fill.

2. **The machine type is a read-off, never a re-derivation.** Nothing downstream infers a type. `lower` and
   `select` obtain a wasm valtype by asking the *already-solved* `Ty` (`core_valtype` / `comp_valtype`).
   There is exactly one notion of a value's type — its solved structure — and its wasm representation is a
   projection of that structure, computed where the structure is complete.

Finalization (`ground`) resolves every node against the completed substitution and treats a *residual*
unsolved variable as a **decline** ("type could not be determined"), never a silent default — with two
principled and narrow exceptions, both of which are cases where the value is fully determined even though a
variable is free: a variable that appears only as a *sum type argument* is a phantom parameter defaulted to
`Unit` (`(None unit)` is `Option a` with `a` free but the value is a determinate `None`), and every free
variable *inside a `Ty::Fn`* is defaulted to `Unit` because a function value is always compile-time-only
and its variables are pinned at the application site (see
[[2026-07-09-const-folding-is-the-one-tier-poison-plus-dce-give-reachability]]).

**Why.** The predecessor seed's "inference" was a coarse wasm-valtype classifier
(`Int64|Bool|Float64|Unit|Never|Heap`, every compound collapsed to one opaque `Heap`), carried alongside a
separate `Shape`, and **re-derived ad hoc during emission** so that a per-function fixpoint and an
emit-time re-derivation had to agree. When they disagreed the compiler shipped an invalid or wrong
component, and a whole family of self-hosting bugs (asks 14/18/24/34/65/73/77 — recursive-Bool branch
order, list-accumulator, fixpoint-OOM, polymorphic-identity-returns-1, shape-lost-across-return,
tail-recursive tuple return) turned out to be *one* fault seen at different lattice points: order-dependent
unification of a placeholder against a concrete type, with no type variables to make the order not matter.
The seed itself diagnosed the general fix in ask-14 — "order-independence is a property every result kind
needs; fix it at general result-unification, not per-kind" — and then patched each lattice point instead,
which is why the family never closed. This learning is the empirical vindication, from a completed rebuild,
of the [2026-07-04 HM decision][[2026-07-04-inference-is-hindley-milner]] and its
[coarse-kind post-mortem][[2026-07-08-a-coarse-kind-classifier-re-derived-at-emit-is-the-wrong-inference-and-fails-one-way-at-every-lattice-point]]:
real type variables plus a commutative unifier make every one of those bugs *structurally impossible*
rather than individually fixed, and "structure is the inferred type" closes the Kind-vs-Shape gap by
construction because there is only one solved type and its shape is its structure.

The reproduction-critical discipline is not "use HM" — the spec already requires principal-type inference
by unification — but the two rules that a fresh implementer will otherwise violate under schedule pressure,
exactly as the seed did: **infer before you lower and materialize the result** (never let a later pass
re-decide a type it could read), and **an undetermined type is a rejection, not a guess** (never default a
residual variable to whatever the emit path finds convenient; a silent default is the coarse classifier
sneaking back in). The `Subst::apply`-recurses-into-compounds and `ground`-uses-recursive-`has_unsolved_var`
details are load-bearing: a shallow version lets a nested unsolved variable survive to render, which is the
same "two disagreeing notions of the type" bug at a smaller scale.

**The requirement it drove.** No new behavioral requirement — this realizes `type-system.md` §"Inference Is
Principal-Type Inference By Unification" (unification over type variables; principal types; propagate to
every occurrence — the order-independence the seed lacked), §"A Let-Bound Definition Is Generalized," and
§"Every Expression Has A Static Type" (an undetermined type is a rejection), together with
`compiler-pipeline.md` §"Emission Serializes A Lowered Representation" (emission "MUST NOT decide a type").
The reproduction content **not yet folded**, for the architecture reference doc: (1) inference is a
*distinct pass that materializes a typed representation*, and every downstream pass *reads* the solved type
rather than re-deriving one — the spec requires resolved *names* before selection but does not yet require
the *type* to be solved-and-read the same way; and (2) *solve-once/read-downstream* stated as the general
anti-pattern-avoidance rule of which the coarse-kind failure is the cautionary instance. The "HM" /
"Algorithm W" name stays out of the capability text by the no-proper-names discipline; it belongs only here
and in the architecture doc.
