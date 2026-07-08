# An unbound name in a short-circuited boolean operand is not scope-checked

*2026-07-08*

**What happened.** Adversarial probing of boolean connectives found that an unbound name in a
short-circuited (unevaluated) operand is not rejected. `(and false undefined-name)` runs to `false` —
the constant left operand `false` short-circuits the conjunction, the right operand `undefined-name` is
never evaluated, and the compiler never resolves it, so the unbound reference slips through. `(or true
undefined-name)` runs to `true` likewise. The evaluated-operand forms are correctly caught: `(and true
undefined-name)` and `(or false undefined-name)` both decline "unbound name: undefined-name". And the
seed already type-checks the dead operand — `(and false (+ 1 1))` rejects "operand is not a Bool",
`(and false (+ 1 true))` rejects "operation on mismatched types" — so only the SCOPE check is missing on
the short-circuited operand.

**Why it is a break.** core-semantics.md #Boolean Connectives Short-Circuit is explicit that the two
forms are identical: "a connective shields a trapping or effectful right operand exactly as the
unselected branch of a conditional does", and "Each operand of a boolean connective MUST be type-checked
as a boolean whether or not it is evaluated, so that an unevaluated operand cannot carry a deferred
error, exactly as every branch of a conditional is type-checked." Combined with #Binding Is Lexical ("A
reference to a name with no enclosing binding MUST be a compile-time error", unconditional), an unbound
name in a short-circuited operand MUST be rejected CDZ0101, exactly as `(if true 1 undefined-name)` is.
Running to `false` is a false-accept of an ill-formed program.

**Root cause (likely) — the connective's dead-operand check covers types but not scope.** The connective
desugars to a conditional (an `and`/`or` becomes a nested `if`), and the seed type-checks both operands
of that conditional whether or not they are evaluated (so the type errors above are caught), but the
unbound-name / scope check added for an unselected `if` branch (the c25-if fix — `provably_unbound_name`
reached from `gen_if`) is not applied to the connective's short-circuited operand: the const-fold of the
connective emits only the taken side and scope-checks only that operand. So the dead operand's type is
checked but its free names are never resolved. The fix is to run the same dropped-branch scope check on a
short-circuited connective operand that the unselected `if` branch already gets — the connective lowers
through the same conditional shielding, so the scope check must reach it identically.

**The lesson (the recurring family).** The spec says a connective operand is checked "exactly as every
branch of a conditional is" — and the TYPE half of that promise is kept while the SCOPE half is not. A
fix proven on one form (the unbound-name check on an unselected `if` branch) must carry to its sibling
(the short-circuited connective operand), especially when the spec explicitly ties the two together. This
is the same "a check proven on one form is not carried to its sibling" shape as the annotation-descent
(tuple/list/sum vs record), the bool/sum-vs-int exhaustiveness, and the nominal record-vs-sum boundary —
here the siblings are the unselected conditional branch and the short-circuited connective operand, which
core-semantics.md declares must behave identically. The tell: the identical unbound name rejects under
`if` and under an evaluated operand but runs under a short-circuited one; and the connective's dead
operand is type-checked but not scope-checked, so the two halves of one spec sentence diverged.

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"an unbound name in a
short-circuited boolean operand is still rejected" — `(and false undefined-name)` MUST reject CDZ0101,
the connective companion of the unselected-conditional-branch case above it. Native seed; the behavior
gate catches it (expected reject CDZ0101, observed a running component). A generation that does not yet
scope-check the short-circuited operand declines rather than answering `false`.
