# A runtime-scrutinee match with a bare-binder arm reinterprets the payload as the other arm's type

*2026-07-08*

**What happened.** A match over a runtime scrutinee whose arms have DIFFERENT types is supposed to be
rejected (a match is an expression of one type, exactly as a conditional's branches must agree). It is
rejected in most forms — but it SLIPS, and worse MISCOMPILES, when the first arm's body is a bare
payload binder:

- `(def (f o) (match o ((Some x) x) ((None _) true)))` over `o : Option Int64` — the `Some` arm body
  `x` is Int64 (the payload), the `None` arm body `true` is Bool — disagreeing arm types, so the match
  is ill-typed and must be CDZ0201. Instead the seed accepts it and reinterprets the payload's bits as a
  Bool across the run boundary: `(f (Some 5))` → **`true`**, `(f (Some 42))` → **`false`**,
  `(f (Some 0))`/`(f (Some 1))`/`(f (Some 2))`/`(f (Some 3))` → `false`. The value is neither the true
  Int payload nor a rejection — the Int64 emerges as a Bool.

**The isolating tell.** The arm-type-agreement check fires for every first-arm shape EXCEPT a bare
binder: a literal first arm `((Some x) 99)` rejects "match arm bodies have different types"; an
arithmetic first arm `((Some x) (+ x 0))` rejects "runtime sum match arms differ in kind"; only the
bare payload-binder first arm `((Some x) x)` slips. And it only miscompiles on a RUNTIME scrutinee — an
inline constant scrutinee `(match (Some 5) ((Some x) x) ((None _) true))` const-folds to the correct
Int `5`. So the defect is exactly the runtime-scrutinee + bare-binder-first-arm path.

**Why it is a break.** core-semantics.md #Matching Is Exhaustive Or Rejected makes a match an expression
whose type is what its arms yield, and #Conditionals Evaluate One Branch requires every branch
type-checked whether or not evaluated — the same for a match's arms. Disagreeing arm bodies (Int64 `x`
vs Bool `true`) are ill-typed, CDZ0201, the match analogue of the conditional branch-agreement cases
(`(if … 1 true)` is rejected) and the existing const-scrutinee match case `(match 5 (5 1) (_ true))`.
This is a wrong VALUE, not only a missed rejection: an Int64 payload crosses the run boundary as a Bool.

**It falsifies the corpus's own assumption.** The existing case "a match whose arm bodies have different
types is a type error even when a constant scrutinee selects one" (02-binding-and-control.sexp) states
in its rationale: "A RUNTIME-scrutinee match already checks this ('runtime match arms differ in kind');
the gap is the const-folded path." This break shows the runtime-scrutinee match does NOT check it when
the first arm is a bare payload binder — the "already checks this" premise is false for that arm shape,
and the failure is not a benign miss but a payload-bit-reinterpretation miscompile.

**Root cause (bare-binder arm skips the result-type comparison).** The runtime-sum-match lowering
derives each arm body's type/kind to check agreement ("arms differ in kind") — but when an arm body is
JUST the payload binder, it appears to take that arm's type directly from the bound payload's slot kind
(here Int64) as the match's result type and never compares it against the OTHER arms' body types. A
literal or expression arm body goes through the ordinary body-typing that runs the agreement check; a
bare binder short-circuits to the payload kind. The `None` arm's Bool body is then emitted into the same
result slot, and the caller reads the Int64 payload through the match's inferred (mismatched) result
representation — the Int bits rendered as a Bool. The tell: swap the bare binder for `(+ x 0)` (same
Int64 value) and the check fires; the bare binder is the only body that bypasses it.

**Fix direction (gitignored seed).** In the runtime-sum-match arm-typing, treat a bare-payload-binder
arm body as an ordinary expression of the binder's type and include it in the arm-result-type agreement
comparison — do not shortcut its type to the payload slot kind without comparing it against the other
arms. Equivalently: compute the join/agreement of ALL arm body types (rejecting on disagreement) before
choosing the match's result representation, regardless of whether an arm body is a bare binder. Regression
guards: a match whose bare-binder arm agrees with the others still works (`(match o ((Some x) x) ((None
_) -1))` on `(Some 5)` → 5, on `(Some 42)` → 42); the literal/arithmetic mismatch arms still reject; the
const-scrutinee arm-agreement case still rejects; a Bool-returning runtime match (`(match n (0 true) (_
false))`) still works.

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"a runtime-scrutinee match with a
bare-binder first arm and a differently-typed second arm is a type error" — `(match o ((Some x) x)
((None _) true))` over a runtime `o` MUST be CDZ0201. Placed right after the existing const-scrutinee
arm-agreement and internally-ill-typed-arm cases, framed as the runtime-scrutinee + bare-binder gap that
falsifies their "runtime match already checks this" assumption. Realized (runtime Option match is
realized), the behavior gate catches it (expected CDZ0201, observed the program runs and reinterprets
the payload as a Bool).

Related: the const-scrutinee arm-agreement case and the internally-ill-typed-arm case (same file, the
premise this falsifies); the conditional branch-agreement cases (#Conditionals Evaluate One Branch);
[[variant-wrong-payload-unchecked-as-match-scrutinee-break]] (c82 — the sibling scrutinee-position check
gap; both are match-typing paths skipping an ordinary type check). Master-pattern family: a check proven
on most arm-body shapes (literal, expression) must carry to the bare-payload-binder arm body too, and the
runtime-scrutinee path is not exempt.
