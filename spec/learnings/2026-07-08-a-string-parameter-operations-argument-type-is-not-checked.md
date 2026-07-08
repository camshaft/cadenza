# A string-parameter operation's argument type is not checked

*2026-07-08*

**What happened.** Adversarial probing of the effect surface found that the perform-argument type check
(the c30 fix) does not fire when the operation's declared parameter type is String. `E.emit` declared
`(-> String Unit)`, performed with an Int64 — `(E.emit 42)` — runs to `unit` inside an intra-program
handler instead of being rejected. Bool and compound arguments run too (`(E.emit true)`, `(E.emit (list
1 2))` → `unit`). The Int64-parameter contrast IS caught: `(E.op true)` and `(E.op "x")` for an
Int64-parameter op decline "perform argument type does not match the declared parameter type". The
smoking gun: with a handler arm that actually uses the bound parameter as a String — `(E.emit 42)` under
`((E.emit (s) st (resume unit (String.byte-len s))))` — the program declines "String.byte-len of a
non-String value", proving the Int `42` was bound where a String was expected and only a downstream
String operation notices.

**Why it is a break.** capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To
The Row: "Performing an operation MUST check its arguments against the operation's declared parameter
types … so that an effect operation is typed exactly as an ordinary function application is." An Int64
argument to a String-parameter operation is a type mismatch, CDZ0201, exactly as the Int64-parameter
case `(E.op true)` is. Running `(E.emit 42)` to `unit` is a false accept of an ill-typed program — the
mistyped value is bound into the handler arm as a String.

**Root cause (likely) — the perform lowering dispatches on the DECLARED parameter type before checking
the argument's actual type, and the String branch skips the check.** The c30 fix added the
argument-vs-parameter check, but the perform/handler lowering appears to route by the operation's
declared parameter type: a String-parameter op is routed to a string-argument path (host-side, that path
is the unrealized "runtime string argument to host call not yet lowered" decline; intra-handler-side, it
binds the argument into the arm typed as String) WITHOUT first verifying the argument is a String. So the
Int64-parameter op reaches the arg-type check but the String-parameter op is dispatched past it. The fix
is to run the argument-type check for every declared parameter type — including String — before any
type-directed lowering dispatch, so `(E.emit 42)` rejects CDZ0201 like `(E.op true)` does.

**The lesson (the recurring family).** A check proven for one parameter type (Int64) is not carried to a
sibling parameter type (String), because a type-directed dispatch routes the String case past the check.
This is the "a check proven on one form is not carried to its sibling" family, here at the granularity of
the operation's parameter TYPE: the argument-type check must be uniform across parameter types, not
gated by which lowering path the declared type selects. The tell: the identical ill-typed perform is
rejected when the parameter is Int64 but runs when the parameter is String — the dispatch on the declared
type, not the value, decided whether the check ran. (Same shape as the earlier bool/sum-vs-int
exhaustiveness and annotation-descent gaps — a per-type branch omitted the check one branch already had.)

**Corpus case added.** `spec/semantics/14-effects-and-handlers.sexp` §"performing a string-parameter
operation with a non-string argument is a type error" — `(E.emit 42)` for `E.emit : String → Unit` MUST
reject CDZ0201, the String-parameter sibling of the Int64-parameter perform-argument case above it.
Gated `(needs effects)`, which the seed realizes, so the behavior gate runs and catches it (expected
reject CDZ0201, observed a running component yielding `unit`). A generation that does not yet check a
String-parameter op's argument declines rather than binding the mistyped value into the handler arm.
