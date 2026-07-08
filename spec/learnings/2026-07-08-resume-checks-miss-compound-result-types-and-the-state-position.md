# Resume checks miss compound result types and the state position

*2026-07-08*

Two sibling gaps in the handler-`resume` checking, both in the lineage of c43 (which added the
resume-value result-type check for scalar results).

## Gap A — the resume-value result-type check is bypassed for a COMPOUND result type

**What happened.** `E.get` declared `(-> (List Int64))` has result type `List Int64`. A handler
resuming with a non-list — `(resume 42 s)` (Int64), `(resume true s)` (Bool), `(resume (tuple 7 8) s)`
(tuple) — is accepted. `(E.get)` returns `42`, `true`, and — worst — `(list)` for the tuple resume: a
TUPLE reinterpreted through the op's List result slot and rendered as an empty list, a type-confusion
wrong value. The SCALAR-result case (c43) is correctly rejected: `(resume true s)` for an Int64-result op
declines "resume value type does not match the declared result type". So the check works for a scalar
result type but is bypassed for a compound one.

**Why it is a break.** capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To
The Row: a perform must "yield the operation's declared result type". An Int64 (or tuple) resumed where
`List Int64` is declared is a result-type mismatch, CDZ0201, exactly as the scalar case is. Yielding `42`
or a tuple-as-`(list)` is a false accept (and, for the tuple, a type-confusion miscompile).

**Root cause (likely).** The c43 fix compares the resume value's type against the op's declared result
type only where the result is a scalar Kind; a compound result type (List/Tuple/Record/sum) is not
compared, so any resume value passes. The fix is to check the resume value against the full declared
result type, compound included, as the annotation-contradiction descent already does for other positions.

## Gap B — an unbound name in the resume STATE position is not scope-checked

**What happened.** `(resume <value> <state>)` carries two expressions. An unbound name in the VALUE
position is rejected (`(resume undefined-xyz s)` → "unbound name: undefined-xyz"), but an unbound name in
the STATE position is not: `(resume unit undefined-xyz)` runs to the handler's result instead of
rejecting.

**Why it is a break.** core-semantics.md #Binding Is Lexical (unconditional): a reference to a name with
no enclosing binding is a compile-time error (CDZ0101). The resume state is an ordinary expression, so an
unbound name in it must be rejected exactly as one in the resume value is. This is the same unbound-name
gap the unselected-conditional-branch (c25-if) and short-circuited-connective-operand (c37) cases closed,
here in a resume's second argument.

**Root cause (likely).** The scope/emit pass walks the resume value but not the resume state (or walks it
in a mode that doesn't resolve names), so a free name in the state is never checked. The fix is to
scope-check both arguments of a resume.

## The lesson (the recurring family)

Both gaps are the "a check proven on one position/type is not carried to its sibling" family, both inside
`resume`: the result-type check landed for scalar result types but not compound (Gap A), and the
unbound-name check landed for the resume value but not the resume state (Gap B). A check on a construct
with multiple typed positions must cover every position and every type shape, not the first/scalar one
only. The tells: the identical wrong-type resume is rejected for a scalar result but runs for a compound
result; the identical unbound name is rejected in the resume value but runs in the resume state.

**Corpus cases added** (`spec/semantics/14-effects-and-handlers.sexp`, both `(needs effects)`):
- §"resuming with a wrong-type value for a compound result type is a type error" — `(resume 42 s)` for
  `E.get : (-> (List Int64))` → CDZ0201 (Gap A).
- §"an unbound name in a resume's state position is rejected" — `(resume unit undefined-xyz)` → CDZ0101
  (Gap B).
Both native seed; the behavior gate catches each (observed a running component). A generation that does
not yet cover these declines rather than yielding/emitting.
