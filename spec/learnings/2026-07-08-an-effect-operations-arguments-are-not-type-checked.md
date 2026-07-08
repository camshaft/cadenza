# An effect operation's arguments are not type-checked

*2026-07-08*

**What happened.** Adversarial probing of the effect surface found that performing an effect
operation does not check its arguments against the operation's declared parameter types.
`E.op` declared `(-> Int64 Int64)`, performed as `(E.op true)`, runs to `true` — a Bool fed to
an Int64-parameter operation. Worse, `(E.op "str")` runs to `7500915` — a garbage integer, a
String reinterpreted through the op's Int64 slot: a wrong-value miscompile, not just a missing
rejection. The exact analogue for an ordinary function — `(f true)` where `f` takes Int64 — is
correctly rejected "operation on mismatched types."

**Why it is a break.** capabilities-and-effects.md #Performing An Operation Is Typed And
Contributes To The Row: "Performing an operation MUST check its arguments against the operation's
declared parameter types and yield the operation's declared result type, so that an effect
operation is typed exactly as an ordinary function application is." So `(E.op true)` on an
Int64-parameter op is a type mismatch, CDZ0201, exactly as `(f true)` is. The compiler checks the
ordinary application and not the effect perform, so the perform is NOT "typed exactly as an
ordinary function application" — and a String argument through an Int64 slot produces a nonsense
value rather than a rejection.

**Root cause (likely) — the perform lowering skips the argument type-check the call lowering has.**
An ordinary application `(f arg)` checks `arg`'s type against `f`'s parameter; the effect-operation
perform `(E.op arg)` lowers the argument and dispatches to the handler/host without the same
parameter-type check against the operation's declared signature. So a Bool or String argument is
passed through the op's declared Int64 slot unchecked, and the mistyped value flows into the handler
(or the op's result) — producing a garbage integer for a String. The fix is to type-check a
perform's arguments against the operation's declared parameter types at the perform site, reusing
the ordinary-application argument-check, so `(E.op true)` rejects CDZ0201 like `(f true)`.

**The lesson (the recurring family, on the effect surface).** "Typed exactly as an ordinary
function application" is a spec promise that the perform site must actually discharge the SAME
check the ordinary application does — and it did not: the argument-type check lives on the call
path but was not carried to the effect-operation-perform path. This is the same "a check proven on
one form is not carried to its sibling" shape as the collection-growth-operator, if/match
unselected-alternative, and int/float separator findings — here the two forms are an ordinary call
and an effect perform, described as identical in typing but implemented with the check on only one.
The tell: the identical wrong-typed argument (`true`, `"str"`) rejects through a function call but
runs (to a value, or garbage) through an effect perform.

**Corpus case added.** `spec/semantics/14-effects-and-handlers.sexp` §"performing an operation with
an argument of the wrong type is a type error" — `(E.op true)` with `E.op : Int64 → Int64` MUST
reject CDZ0201 (gated `(needs effects)`, which the seed realizes), the perform-site companion of
the ordinary-application argument-type check. Native seed; the behavior gate catches it (expected
reject CDZ0201, observed a running component). A generation that does not yet type-check a perform's
arguments declines rather than emitting the mistyped operation.
