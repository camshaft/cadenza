# A resume value is not checked against the operation's result type

*2026-07-08*

**What happened.** Adversarial probing of the effect surface found that a handler's resume value is not
checked against the operation's declared result type. `E.op` declared `(-> Int64 Int64)` has result type
Int64, but `(resume true s)` — resuming with a Bool — is accepted, and `(E.op 1)` yields `true`. The
opposite mismatch runs too: for a Bool-result op, `(resume 99 s)` yields the integer `99`. The
argument-type half of the same rule IS enforced (`(E.op true)` on an Int64-parameter op is rejected —
the c30 fix), and when the perform result flows into a typed context the use site incidentally catches
it (`(+ (E.op 1) 1)` declines "non-integer operand"), but the resume value itself is not checked, so a
perform whose result is not otherwise constrained yields the wrong-typed value.

**Why it is a break.** capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To
The Row: "Performing an operation MUST check its arguments against the operation's declared parameter
types AND YIELD THE OPERATION'S DECLARED RESULT TYPE, so that an effect operation is typed exactly as an
ordinary function application is." A handler arm resumes with the value the operation yields —
`(resume <value> <state>)` "returns <value> to the point that performed the operation" (the effects
corpus header) — so the resume value IS what the operation yields, and it must have the declared result
type. `(resume true s)` for an Int64-result op violates "yield the operation's declared result type"
exactly as an ordinary function whose body returns a Bool where its signature says Int64 would. This is
the result-type half of the very sentence whose argument-type half was fixed as c30.

**Root cause (likely) — the perform lowering type-checks arguments but not the resume value against the
result type.** The c30 fix added the argument-vs-parameter check at the perform site, but the dual check
— the resume value in a handler arm against the operation's declared result type — was not added. So a
handler arm can resume with any-typed value, and the perform's result carries whatever the arm resumed
with, unchecked against the op's declared result. The fix is to type-check each handler arm's `resume`
value against the arm's operation's declared result type (and the handler's fall-through/return value
against the handled expression's type), rejecting CDZ0201 on a mismatch — so the perform "yields the
declared result type" as the spec's second clause requires.

**The lesson (the recurring family).** One spec sentence states two obligations — "check arguments
against parameter types AND yield the declared result type" — and only the first was implemented. A fix
must discharge the WHOLE rule it cites, not one clause of it: c30 pinned and fixed the argument half, but
the result half of the identical sentence stayed open. This is the "a check proven on one form/aspect is
not carried to its sibling" family, here at its tightest — the two siblings are the two clauses of a
single MUST. The tell: `(E.op true)` (bad argument) rejects, but `(resume true s)` (bad result, same op,
same sentence) runs to `true`; the perform is "typed exactly as an ordinary function application" on its
inputs but not on its output.

**Corpus case added.** `spec/semantics/14-effects-and-handlers.sexp` §"resuming with a value of the wrong
type for the operation's result is a type error" — a handler arm `(resume true s)` for `E.op : Int64 →
Int64` MUST reject CDZ0201, the result-type companion of the perform-argument case above it. Gated
`(needs effects)`, which the seed realizes, so the behavior gate runs and catches it (expected reject
CDZ0201, observed a running component yielding `true`). A generation that does not yet check the resume
value against the result type declines rather than yielding it.
