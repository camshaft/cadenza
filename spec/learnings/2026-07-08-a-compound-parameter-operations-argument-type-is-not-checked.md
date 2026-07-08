# A compound-parameter operation's argument type is not checked

*2026-07-08*

**What happened.** Adversarial probing completed the matrix of the effect-operation type-check
blind spot. The perform-argument check does not fire when the operation's declared parameter type is a
COMPOUND. `E.put` declared `(-> (List Int64) Unit)`, performed with an Int64 — `(E.put 42)` — runs to
`unit` instead of rejecting. A Bool argument, a tuple where a list is declared, and a list of the wrong
element type all run the same way. The smoking gun matches c48: a handler arm that uses the bound
parameter as a list — `(E.put 42)` under `((E.put (xs) s (resume unit (List.len xs))))` — declines
"List.len of a non-list value", proving the Int `42` was bound into the arm where a `List Int64` was
expected. The scalar-parameter contrast IS caught: `(E.op true)` for an Int64-parameter op declines
"perform argument type does not match the declared parameter type" (the c30 fix).

**Why it is a break.** capabilities-and-effects.md #Performing An Operation Is Typed And Contributes To
The Row: "Performing an operation MUST check its arguments against the operation's declared parameter
types … typed exactly as an ordinary function application is." An Int64 argument to a `List Int64`
parameter is a type mismatch, CDZ0201. Running `(E.put 42)` to `unit` is a false accept of an ill-typed
program.

**Root cause — the effect operation type-check compares only against scalar Kinds.** This is the same
root as c48 (String parameter) and c49a (compound RESULT type): the effect operation's argument/result
type check was written to compare against a scalar Kind (Int64/Bool), and every non-scalar declared type
— String (c48) and compound List/Tuple/Record/sum (this case on the argument side, c49a on the result
side) — is dispatched past the check, binding/yielding the mistyped value. The four cases form one
matrix:

  |            | scalar param/result | String | compound |
  | argument   | ✓ rejected (c30)    | ✗ c48  | ✗ THIS   |
  | result     | ✓ rejected (c43)    | —      | ✗ c49a   |

The fix is one change with broad reach: the effect operation type-check must compare the argument against
the FULL declared parameter type and the resume value against the FULL declared result type — scalar,
String, and compound alike — reusing the same type-comparison the annotation-contradiction descent and
ordinary function application already apply, rather than a scalar-Kind-only comparison.

**The lesson (the recurring family).** A type-check written for the scalar case and not generalized to
String/compound leaves every non-scalar type as a silent hole. "Typed exactly as an ordinary function
application" must hold for every argument and result type shape, not the scalar ones the check happened to
cover first. The tell: the identical ill-typed perform is rejected when the parameter is Int64 but runs
when it is String or a compound — the type of the declared parameter, not the ill-typedness, decided
whether the check ran.

**Corpus case added.** `spec/semantics/14-effects-and-handlers.sexp` §"performing an operation with a
wrong-type argument for a compound parameter is a type error" — `(E.put 42)` for `E.put : (-> (List
Int64) Unit)` MUST reject CDZ0201, the compound-parameter sibling of the Int64-parameter (c30) and
String-parameter (c48) argument cases. Gated `(needs effects)`, which the seed realizes, so the behavior
gate runs and catches it (expected reject CDZ0201, observed a running component yielding `unit`). A
generation that does not yet check a compound-parameter op's argument declines rather than binding the
mistyped value.
