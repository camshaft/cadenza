# A language with conditionals still needs boolean connectives — the spec had none

*2026-07-06*

**What happened.** Authoring the compiler in Cadenza, a routine predicate — the signed-LEB128
terminator, "the shifted remainder is all sign bits *and* the current group's sign bit agrees, *or*
the remainder is all ones *and* the sign bit is set" — could not be written. `(and a b)`, `(or a b)`,
and `(not a)` all compiled to `declined: undeclared capability: and`. A grep confirmed the hole was
total: logical connectives appeared in no seed lowering, no conformance case
(`spec/semantics/*.sexp`), and no capability requirement. The language had `if` (with a proven
one-branch-only evaluation guarantee), the comparison operators, and `Bool` with a total order — but
no way to *combine* two booleans without nesting a conditional per condition. The predicate had to be
hand-desugared to `(if A (if B true false) false)`, which compiled and passed every known-answer
LEB128 case, confirming the workaround was sound and the connectives were the only thing missing.

**Why.** The gap survived because nothing upstream of an actual program ever pressed on it. The
conformance corpus grew case by case from the mandatory floor outward, and no case happened to need a
logical connective; the `if` short-circuit guarantee and the comparison operators were each specified
in isolation, so "compose two conditions" fell in the crack between them. Boolean connectives are so
basic that their absence reads as impossible rather than as an omission to check for — precisely the
kind of hole a clean-room specification grown outward from a floor leaves, because a floor enumerates
what must be present and says nothing about what a working programmer will reach for on the first
non-trivial predicate. Writing a real program (the compiler) rather than another isolated corpus case
is what applied the pressure that surfaced it.

**The requirement it drove.** Added a *Boolean Connectives Short-Circuit* section to
`spec/capabilities/core-semantics.md` under Control Flow, adjacent to *Conditionals Evaluate One
Branch* because a connective is defined by the same short-circuit discipline: the language MUST offer
conjunction, disjunction, and negation over booleans; conjunction MUST evaluate its right operand only
when the left is true and disjunction only when the left is false (so a connective shields a trapping
or effectful right operand exactly as an unselected conditional branch does); and each operand MUST be
type-checked as a boolean whether or not it is evaluated, mirroring the existing rule that every branch
of a conditional is type-checked. The short-circuit choice — rather than eager evaluation of both
operands — is the load-bearing decision, because it fixes the connective's behavior on a right operand
that traps or performs an effect, making `(or done (perform …))` and `(and present (index xs i))` mean
what a programmer expects. A conformance case witnessing both the value table and the shielding of a
trapping right operand accompanies the requirement.
