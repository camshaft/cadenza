# A variant with a wrong-type payload is unchecked as a direct match scrutinee

*2026-07-08*

**What happened.** Constructing a sum variant with a payload of the wrong type — `(I true)` under
`(type N (I Int64 | J Int64))`, where `I`'s declared payload is Int64 and `true` is Bool — is correctly
rejected in **every** position EXCEPT when the constructor is written directly as the scrutinee of a
`match`. There, the payload type-check is suppressed and the program runs, binding the arm's variable to
the ill-typed Bool and returning it:

- `(match (I true) ((I x) x) ((J y) y))` → **`true`** (a Bool crossing the run boundary where the arm's
  Int64 payload `x` is required — a wrong value).
- `(match (S 99) ((S x) x) ((K y) y))` under `(type N (S String | K Int64))` → **`99`** (an Int where a
  String payload is required) — same defect, different payload types, so it is not Int/Bool-specific.

Every OTHER position rejects `(I true)` with "a unary variant applied to a payload of the wrong type":
bare `(I true)`; let-bound-and-returned `(let ((n (I true))) n)`; let-bound-**then-matched** `(let ((n (I
true))) (match n …))`; as a function argument `(f (I true))`; annotated `(: (I true) N)`; over-applied
`(I 5 6)`. Only the constructor written **directly** in scrutinee position slips.

**Why it is a break.** A sum's shape is "its variant names with their payload types" (type-system.md #The
Structural Types Are Record, Tuple, And Sum), and a value of a sum type "MUST be constructed through one
of its variants" — construction that supplies a payload of the wrong type is ill-typed, no matter the
surrounding context, and must be rejected (CDZ0201). This is a genuine WRONG VALUE, not merely a missed
rejection: `x` is bound to the payload of `I Int64`, so the arm's result type is Int64, yet the program
returns the Bool `true`. The corpus already pins the payload-type check descending through annotations
(option payload, nested option, list element, record field — all CDZ0203) and firing at construction
(the seed's "unary variant applied to a payload of the wrong type"). The direct-match-scrutinee position
is the one place it does not fire.

**Root cause (the master pattern — a check proven on many positions, missed on the match scrutinee).**
The seed type-checks a constructor application's payload against the variant's declared type wherever the
constructor's value is produced and used — as an expression to return, a let value, an argument, an
annotated value. But when the constructor is the immediate scrutinee of a `match`, the match's
scrutinee-typing path evidently determines the scrutinee's *sum type* (to drive exhaustiveness and bind
the arms) WITHOUT running the ordinary constructor-application payload check on it — it trusts the
variant tag and binds the arm variable to whatever payload was supplied. A let-bound scrutinee is checked
because the `let` value goes through ordinary expression checking first; the inline scrutinee bypasses
that. The tell: `(let ((n (I true))) (match n …))` rejects, but `(match (I true) …)` — the same
constructor, one binding removed — runs and returns the ill-typed payload.

**The lesson (a scrutinee is an ordinary expression and must be type-checked as one).** A match's
scrutinee is an expression like any other; determining its sum type for exhaustiveness does not remove
its obligation to be well-typed, including the payload types of any constructor it is. The check that
fires for the constructor in every construction/binding/argument/annotation position must also fire when
the constructor sits in scrutinee position. Same family as the recurring "check-descends-to-leaves" and
"a mechanism proven on one position must carry to every sibling position" bugs: here the position is the
direct match scrutinee, and the check skipped is the constructor-payload type check.

**Fix direction (gitignored seed).** In the match type-checking path, type-check the scrutinee
expression with the ordinary expression checker (which already rejects a wrong-payload constructor)
BEFORE — or in addition to — deriving its sum type for exhaustiveness and arm binding. Equivalently:
route a constructor-application scrutinee through the same payload-type check used for a constructor in
value position, rather than reading its variant tag directly. Regression guards: a well-typed scrutinee
constructor still matches (`(match (I 5) ((I x) x) ((J y) y))` = 5); a let-bound wrong-payload scrutinee
still rejects; a valid match on a runtime/parameter sum still works; exhaustiveness still fires.

**Corpus case added.** `spec/semantics/07-type-system.sexp` §"a variant with a wrong-type payload as a
direct match scrutinee is a type error" — `(match (I true) ((I x) x) ((J y) y))` under `(type N (I Int64
| J Int64))` MUST be CDZ0201. Placed in the payload-type-check cluster (after the record-field-payload
case), framed as the match-scrutinee-position gap. Realized (user sum construction + match are realized),
the behavior gate catches it (expected CDZ0201, observed the program runs and returns Bool `true`).

Related: the annotation payload-descent cases (07-type-system.sexp — option/list/record payload,
CDZ0203); the "check-descends-to-leaves" / scope-at-emit family; the fixed-field/variant-set discipline
(05-compound-types.sexp duplicate-variant case); [[absent-record-field-access-traps-instead-of-static-reject-break]]
(the prior cycle's compound-access check gap). Master-pattern family: a well-formedness check proven on
several positions must carry to the direct-match-scrutinee position.
