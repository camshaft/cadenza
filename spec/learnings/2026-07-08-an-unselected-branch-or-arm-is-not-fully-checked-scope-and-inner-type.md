# An unselected branch or arm is not fully checked — scope and inner-type errors slip through

*2026-07-08*

**What happened.** After the const-folded-match arm-type-agreement fix (c23) and the if-branch
type-agreement fixes (c9/c20), two residual holes remain in the "check every alternative even the
unevaluated one" discipline:

1. **Unbound name in an unselected `if` branch.** `(if true 1 undefined-name)` runs to `1` — the
   else-branch references the unbound `undefined-name`, but the const-folded conditional scope-checks
   only the taken branch. `(if false undefined-name 1)` likewise. The `if` form DOES catch a *type*
   error in the same unselected branch (`(if true 1 (+ 1 true))` → rejected), so the type check
   reaches the dropped branch but the SCOPE check does not.
2. **Internally ill-typed unselected `match` arm body.** `(match 5 (5 1) (_ (+ 1 true)))` runs to
   `1` — the unselected `_` arm's body `(+ 1 true)` mixes Int64 and Bool, an internal type error, but
   the const-folded match takes the unselected arm's RESULT type superficially (Int64, which agrees
   with the selected arm) without type-checking the body. Also `(match 5 (5 1) (_ undefined-name))`
   → `1` (unbound in unselected arm). The selected arm's `(+ 1 true)` IS caught, and the c23 fix
   catches arm-type-DISAGREEMENT (`(_ true)` → reject), but an internally-ill-typed-but-result-type-
   agreeing unselected arm slips through.

**Why they are breaks.** core-semantics.md #Binding Is Lexical: "A reference to a name with no
enclosing binding MUST be a compile-time error" (unconditional). #Conditionals Evaluate One Branch:
"Every branch … MUST be type-checked whether or not it is evaluated, so that an unevaluated branch
cannot carry a deferred error" — and a match's arms are the same kind of alternatives. So an unbound
name or an internal type error in a dropped branch/arm is a deferred error the rule forbids; the
program must be rejected (CDZ0101 / CDZ0201), not run to the folded value.

**Root cause — the checks over alternatives are partial, each covering a different subset.** The
const-fold selects one branch/arm and emits only it; the surrounding checks that should cover ALL
alternatives were added piecemeal and each reaches a different subset:
- `if`: unselected branch gets a TYPE check (catches `(+ 1 true)`) but NOT a SCOPE check (misses
  `undefined-name`).
- `match`: unselected arm gets a RESULT-TYPE-agreement check (c23, catches `(_ true)` vs an Int arm)
  but NOT a full body type-check (misses `(+ 1 true)`) and not a scope check (misses
  `undefined-name`).
The fix is to run the FULL well-formedness pass — scope resolution AND complete type-checking — over
every branch and every arm body, independent of the const-fold, rather than a targeted subset per
construct. One "check this expression fully" applied to each alternative closes all four holes at
once (if-type ✓ already, if-scope, match-arm-type-agreement ✓ already, match-arm-body, match-arm-
scope).

**The lesson (the recurring family, now at its finest grain).** "Check every alternative whether or
not evaluated" is not one check but a bundle — scope resolution, arm/branch result-type agreement,
and each body's internal type-checking — and the compiler discharged them piecemeal, so each new
fix (if-type, match-arm-agreement) closed one facet while leaving siblings open. A const-fold that
emits one alternative must be preceded by the SAME full check the emitted alternative gets, applied
to every alternative — not a bespoke partial check per construct. The tell: the same dropped branch
that rejects a type error accepts an unbound name (if), and the same dropped arm that rejects a
type-mismatched result accepts an internally-ill-typed body (match).

**Corpus cases added.** `spec/semantics/02-binding-and-control.sexp` §"an unbound name in an
unselected conditional branch is still rejected" (`(if true 1 undefined-name)` → CDZ0101) and §"an
internally ill-typed unselected match arm body is a type error" (`(match 5 (5 1) (_ (+ 1 true)))` →
CDZ0201), the scope/inner-type companions of the existing branch- and arm-agreement cases. Native
seed; the behavior gate catches both (observed a running component). A generation that does not yet
fully check dropped branches/arms declines rather than emitting the folded one.
