# An annotation checks the head constructor but accepts any payload type

*2026-07-07*

**What happened.** Adversarial probing of the type-annotation form found a wrong-accept:
`(: (Some true) (Option Int64))` compiled and ran, returning `(Some true)`. But `Some true`
has type `Option Bool`, which cannot unify with the annotated `Option Int64` — the program is
ill-typed and MUST be rejected. The sibling head-level mismatches are correctly rejected
(`(: (Some 5) Bool)` → CDZ0203, `(: (tuple 1 2) Int64)` → CDZ0203); only the *payload* of a
parameterized type slips through. `(: (Some 5) (Option Bool))` and `(: (Some "x") (Option
Int64))` are accepted the same way.

**Why it is a break.** type-system.md #Annotations Constrain, Never Contradict: "A program
whose annotation cannot be unified with the type inference determines MUST be rejected rather
than have the annotation silently replace the inferred type." Inference gives `Some true :
Option Bool`; the annotation `Option Int64` does not unify; the program must be rejected. The
compiler instead runs it — the silent annotation-replaces-inference the section forbids. This
is the strongest break class: not a debatable value, but an ill-typed program *accepted and
executed*.

**Root cause — the annotation predicate collapses the type to its head.** In the seed
(`codegen.rs::matches_annotation`), the `StaticType::Sum` arm is
`StaticType::Sum => !is_scalar_type_name(ann)` — "a sum value satisfies any non-scalar
annotation." And `type_name((Option Int64))` returns just `"Option"`, dropping the parameter.
So the check confirms only that the head `Option` is not a scalar name (true) and accepts; the
payload `Bool` is never compared to `Int64`. Same for tuple/list/record/map arms — none descend
into the annotation's type parameters.

**Why it is a miscompile, not an honest decline.** The code comment frames the gap as
reject-don't-miscompile: "A compound value annotated with a compound or unknown annotation is
not checked here (a not-yet-checked rule is not a rejection)." But leaving a rule unchecked
must produce a *decline* (graded todo), not an *accept* that runs the program. `matches_annotation`
returning `true` for an unchecked case is the bug: an unchecked payload should route to
`decline`, so the ill-typed program neither compiles to a wrong-typed value nor is falsely
rejected. Returning `true` (accept) is the one outcome reject-don't-miscompile rules out.

**The lesson.** A structural predicate that answers a *three-way* question (satisfies /
contradicts / not-yet-known) with a *two-way* boolean has to choose which bucket the unknown
falls into — and "accept" is the wrong default under reject-don't-miscompile. Here
`matches_annotation : bool` folded "not-yet-checked" into "satisfies," turning every unchecked
compound annotation into a silent accept. The safe shape is `satisfies | contradicts | unknown`,
mapping `unknown` to a decline. The head-vs-parameter split is the giveaway: the checker was
written to reject head-level scalar contradictions (which it does) and never grew the recursion
into type parameters, but its boolean return made the missing recursion an *accept* rather than
a *decline*.

**Corpus case added.** `spec/semantics/07-type-system.sexp` §"an option value annotated with the
wrong payload type is rejected" — `(: (Some true) (Option Int64))` MUST reject CDZ0203 (a
generation may decline until it covers the payload check; accepting is the failure). Native seed
only. The behavior gate catches it (expected reject CDZ0203, observed a running component).
