# Tuple-access arity on a function-returned tuple traps instead of rejecting

*2026-07-08*

**What happened.** Adversarial probing of positional tuple access found that the static-arity range check
does not reach a tuple whose arity is known through a FUNCTION RETURN. `(def (mk) (tuple 1 2))` returns a
two-element tuple, so `(tuple.2 (mk))` names position 2 — outside the arity 0..1 — but instead of
rejecting at compile time it emits a component that TRAPS at run time. The directly-written literal
(`(tuple.3 (tuple 10 20 30))`) and the let-bound form (`(let ((p (tuple 1 2))) (tuple.2 p))`) both
correctly reject CDZ0201; only the fn-return form traps. The valid access `(tuple.1 (mk))` works, so the
compiler DOES recover `mk`'s return arity at the projection site — it just doesn't range-check the index
against it.

**Why it is a break.** type-system.md #A Tuple Is Split At A Position Into A Prefix And A Suffix: "a
positional tuple access whose index is out of the tuple's static arity [MUST be] rejected" at compile
time; a tuple's arity is part of its type. The existing corpus case for the literal form states the
principle exactly — "the compiler knows this statically and MUST reject it (CDZ0201) rather than emit a
component that traps at run time … A compile-time-knowable ill-typing must not be deferred to a runtime
trap." `(tuple.2 (mk))` is precisely a compile-time-knowable ill-typing deferred to a runtime trap: `mk`'s
return type is a known 2-tuple.

**Root cause (likely) — the arity range check fires on a literal/let-bound tuple but not on a fn-return
tuple.** The tuple accessor's index range check consults the operand's static arity, which is available
for a literal and a let binding but is recovered on a different path for a function return
(`[[ask65-payload-through-return-resolve-not-inference]]` — resolve reconstructs the tuple through
beta-reduction). That resolve path yields the tuple shape (so a valid `.1` projects correctly) but the
accessor's range check is not applied to the resolved arity, so an out-of-arity index falls through to the
runtime `tuple.N` primitive and traps. The fix is to apply the same index-vs-arity range check to a
fn-return (resolved) tuple's arity that the literal/let path already applies, rejecting CDZ0201.

**The lesson (the recurring family — the master pattern).** A check proven on one form of a construct
(literal and let-bound tuple) is not carried to a sibling form (fn-return tuple) — even though the arity
is equally known there. This is the "a check proven on one variation must carry to every sibling
variation" master pattern, here across how the operand's arity is OBTAINED (literal / let / fn-return).
And it manifests as the worse outcome: the covered forms decline, the uncovered form traps — a
compile-time-knowable ill-typing deferred to a runtime trap. The tell: the identical out-of-arity access
`(tuple.2 …)` rejects on a literal/let tuple but traps on a fn-return tuple. (Distinct from a tuple
reached through a PARAMETER, whose arity is genuinely unknown in the callee body and which correctly
declines "unknown tuple shape".)

**Corpus case added.** `spec/semantics/05-compound-types.sexp` §"a tuple access out of arity on a
function-returned tuple is a type error, not a trap" — `(tuple.2 (mk))` for `(def (mk) (tuple 1 2))` MUST
reject CDZ0201, the fn-return companion of the literal static-arity case above it. Gated `(needs
sum-type-declaration)` (the module-form gate the other fn-return tuple cases use), which the seed
realizes, so the behavior gate runs and catches it (expected reject CDZ0201, observed a runtime trap). A
generation that does not yet range-check a fn-return tuple's access declines rather than emitting the
trapping access.
