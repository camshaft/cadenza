# A payload-bound compound's shape is recovered only inside its match arm, not through a bare function return

*2026-07-08*

**What happened.** A spike ported HOL Light's trusted kernel (`fusion.ml`, the ~680-line LCF
core of types, terms, and theorems) to Cadenza to ask whether the LCF discipline could be
expressed *purely* — no mutable references, inference rules as ordinary functions over an
abstract theorem type. The data core mapped almost directly: `HolType`, `Term`, and
`Thm = Sequent(hyps, concl)` as recursive sums, `type_of` and structural `term_eq` as recursive
walks, and `REFL`/`ASSUME`/`TRANS` as pure functions. The kernel compiled to a valid component
and proved a theorem — `TRANS (REFL x) (REFL x) ⊢ x = x`, verified structurally — but only after
routing around a cluster of declines. The sharpest one: the natural HOL accessor
`concl : Thm → Term`, which matches `(Thm.Sequent (tuple _ c))` and *returns the bound conclusion
`c`* for a caller to inspect, made the whole program decline. Extracting the conclusion *inline*
in the arm that needed it — `(match th ((Thm.Sequent (tuple _ c)) …use c here…))` — compiled and
ran. Reducing it further isolated the boundary precisely: a compound bound out of a sum payload is
matchable and projectable *within the arm that binds it*, but the same value **returned from a
helper** and then projected with `tuple.N` at the call site is not merely declined — the seed
*rejects* it with `CDZ0201: tuple access on a non-tuple`, asserting a well-typed program is
ill-typed.

**Why.** A value bound from a sum payload carries its static shape as arm-local information: the
match that binds it knows the payload's tuple arity / record fields / inner variant, and the
existing payload-shape-threading machinery (`infer_sum_payload_override`, `shape_of`, the
arm-unification that recovers a binder's kind) reconstructs that shape *at the binding site*. A
bare `(def (concl th) (match th ((Sequent (tuple _ c)) c)))` erases it: the function's return
type is inferred as an opaque heap handle, so at the call site the returned value has no recorded
shape and a `tuple.N` (or a `match`) finds nothing to index. The shape was never *wrong* — it was
*not propagated across the function-return boundary*. This is the same family as the runtime
tuple-result cases that once emitted invalid components before the projection learned to recover
the operand's shape at the projection site; the payload-through-return path is the corner that
recovery does not yet reach.

**Why it is a break, not an honest decline.** For a shape it cannot yet thread, the compiler must
*decline* — refuse to derive a component, graded `todo` — never reject a valid program as a type
error (reject-don't-miscompile,
[`2026-07-03-decline-do-not-miscompile.md`](./2026-07-03-decline-do-not-miscompile.md)). Emitting
`CDZ0201` here is the wrong-rejection mirror of a wrong-value: the program
`(is-var (tuple.1 (unbox (Box.B (tuple (list) (Term.Var 7))))))` is well-typed and must yield its
projected element, so a coded rejection is a defect the corpus pins against — exactly as an
unchecked annotation that *accepts* an ill-typed program is a defect. A three-way question
(projectable / not-a-tuple / shape-not-yet-known) answered as a two-way one (project / reject)
folded "shape-not-yet-known" into "reject" instead of "decline."

**The lesson, and its LCF-design consequence.** Where a compound value's *shape* is consumed
matters as much as *that* it is consumed: the shape lives at the binding site and does not
automatically survive a bare return. An abstract-data-type accessor that hands its payload back to
a caller — the ordinary LCF shape, `concl`/`dest_thm`/`dest_eq` returning a term for the caller to
take apart — is precisely the pattern that hits this, while a kernel that destructures its theorem
*inline in the arm that needs the conclusion* compiles today. The general fix is to thread a
payload-bound compound's shape through a function's return type (making the accessor return a
shaped value, not an opaque handle); until then, consume the payload where it is bound. A checker
that cannot yet recover a shape must route the unknown to a decline, never to a coded rejection.

**Corpus cases added.** `spec/semantics/05-compound-types.sexp`: §"a tuple payload extracted
through a helper return must not be rejected as a type error" (the well-typed program the seed
wrongly rejects — the behavior gate catches it: expected the projected element, observed a
`CDZ0201` rejection) and its control §"a tuple payload consumed INLINE in the sum arm projects and
re-matches" (the route that compiles). Two companion gaps the same spike surfaced were recorded
alongside: §"a constructor pattern in a tuple payload slot is matched in one arm" (a nested-ctor
binder occupying a tuple slot, which declines) and, in
`spec/semantics/03-equality-and-observation.sexp`, the runtime compound / two-runtime-string `=`
cases (a heap-walk comparison not yet emitted, with a hand-written recursive comparator as the
route that compiles).
