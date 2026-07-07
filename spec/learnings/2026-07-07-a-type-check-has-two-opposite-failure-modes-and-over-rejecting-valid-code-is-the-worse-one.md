# A type-check has two opposite failure modes, and over-rejecting valid code is the worse one — an unprovable kind must default to silence, not to rejection

*2026-07-07*

**What happened.** The self-hosted compiler's diagnostics handler was activated (`compile` became the
`Diag`-handler artifact-ABI body, no longer bare-`Bytes`), so the byte gate finally exercised the `check-node`
type pass end-to-end. Under the byte gate's new runtime-behavior classifier (ask-33): 79 agree / 102 disagree /
25 soft / 371 decline. Reading the 102 disagrees by native outcome split them two ways: 88 were `native=rejected`
comp=ok (the compiler ACCEPTS ill-typed programs native rejects — the known ask-30 under-rejection), but **9 were
`native=ok` comp=`diagnostics[CDZ0201]`** — well-typed programs native compiles that the check FALSE-REJECTS.

All 9 shared a shape: a Bool whose kind is not statically provable where the check looks at it — a function
PARAMETER used as an `if`/`and`/`or` condition (`(def (f b) (if b 10 20))`, `(def (row a b) (if (and a b) 1 0))`),
a Bool-returning recursive CALL as a condition, or a runtime MATCH scrutinee. The prior, thorough analysis of this
same ask had concluded "the leak is specific to compound operands; the Bool controls all emit correctly" — but its
Bool controls were LITERALS (`(+ 1 true)`, `(if true 1 false)`), which carry a statically-known kind. Re-probing
the aggregate against parameter/call-result Bools flipped the conclusion: **literal Bool is fine; unknown-kind
Bool is over-rejected.**

**Why.** Two lessons, one general and one about method.

The general one: **a type-check has two opposite failure modes, and they are not symmetric in cost.** It can
UNDER-reject (accept a program that should be rejected) or OVER-reject (reject a program that should be accepted).
The reject-don't-miscompile ordering the loop lives by (wrong-value < crash < decline < correct) puts these on
opposite sides: an under-reject is a missing rejection the compiler will eventually add (it accepts a bad program,
which is ask-30's already-tracked territory), but an over-reject is the compiler declaring a GOOD program bad —
a false rejection, which is strictly worse because it denies a correct program its meaning. Here the same coarse
`Kind = Ki64 | KBool` lattice caused both: it mislabels a compound as `Ki64` (under-reject at arith positions) AND
it has no way to say "I don't know this operand's kind," so an operand it can't prove is a Bool falls through as a
mismatch and emits (over-reject at Bool positions). The fix is not one kind but two, of opposite character:
`KCompound` (a representable kind that IS a mismatch at scalar positions — makes the check emit MORE) and
`KUnknown` (a not-positively-known kind that is NEVER an emit trigger — makes the check emit LESS). **The
conservative-check principle is asymmetric: emit ONLY when you can POSITIVELY prove a mismatch; an operand whose
kind you cannot prove must default to SILENCE, never to rejection.** A checker that defaults unprovable to
"error" is not conservative, it is reckless — it converts every gap in its own knowledge into a false rejection of
the user's code.

The method one: **an aggregate conclusion is only as good as the inputs that were sampled to reach it, and "X is
fine" from same-flavored controls must be re-probed with a different flavor before it is trusted.** The prior
analysis wasn't wrong about what it tested — literal-Bool operands really do check correctly — it was wrong to
generalize "Bool is fine" from literals to all Bools, because a parameter's kind reaches the check by a different
path than a literal's. This is the loop's standing rule (a handoff doc's characterization is an aggregate to
re-probe, not inherit) applied to a NEGATIVE claim: "this class works" is exactly as much an aggregate as "this
class fails," and the cheap re-probe — swap the literal for a parameter — is what surfaced the entire
over-rejection half that the aggregate had hidden.

**The requirement it drove.** No new corpus case — the 9 well-typed Bool-parameter programs are ALREADY in the
corpus (that is how the byte gate scored them `native=ok` against the compiler's `diagnostics`); they are the
regression guard, and they will flip from `disagree` to `agree`/`soft` when the over-rejection is fixed, with no
new case needed. The output is the fourth-probe record on ask-53 (the over-rejection half, with the 9 cases and
the unknown-kind root, added alongside the pre-existing under-rejection analysis) and this learning. The concrete
reason `compile` must stay bare-`Bytes`: the activated handler currently false-rejects 9 well-typed programs, and
shipping it would turn those corpus cases red. General lesson: **a type-check fails in two opposite directions;
over-rejecting valid code is worse than under-rejecting invalid code (it denies a correct program its meaning), so
an operand whose kind cannot be positively proven must default to silence — and when an analysis concludes "this
class is fine," re-probe it with a differently-shaped input of the same class before trusting the negative, because
a same-flavored control can hide the entire opposite failure mode.**
