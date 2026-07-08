# A constant-condition `if` inside a function body drops the branch-type-agreement check

*2026-07-08*

**What happened.** The conditional branch-type-agreement check — which rejects `(if … <A> <B>)` when
the two branches have different types — is entirely SKIPPED when the `if` has a compile-time-constant
condition and sits inside a `def`ed function body (any function other than the top-level `main`
expression). The identical mismatch at the top level is correctly rejected; moved into a function, it
slips, and depending on the surviving branch it either silently runs an ill-typed program or emits an
invalid wasm component.

- `(def (f) (if true 1 false))` — Int64 then / Bool else, a type mismatch — is ACCEPTED; `f` returns 1
  (and composes: `(+ (f) 0)` = 1). The identical `(if true 1 false)` as the top-level entry expression
  is correctly rejected "conditional branches have different types" (CDZ0201).
- `(def (f n) (if true (+ n 1) false))` — the surviving then-branch `(+ n 1)` is a COMPUTED expression
  that cannot fold to a literal — emits an **INVALID wasm component** (fails validation:
  `wasm[0]::function[0]`), not merely a wrong value. The unchecked Int/Bool branch-representation
  mismatch reaches code generation.
- The runtime-condition form `(def (f n) (if (> n 0) (+ n 1) false))` is correctly rejected ("if
  branches differ in kind") — both branches are kept and checked. Only the CONSTANT-condition path in a
  function body slips.

**Why it is a break.** core-semantics.md #Conditionals Evaluate One Branch: "Every branch of a
conditional MUST be type-checked whether or not it is evaluated, so that an unevaluated branch cannot
carry a deferred error." A conditional whose branches have different types has no single type and is
ill-typed (CDZ0201) — the corpus pins this for the top-level cases (`(if true 1 false)`, `(if false
(record (a 1)) 7)`, tuple-arity/element mismatches). The rule is unconditional on where the `if` sits;
skipping it inside a function body admits an ill-typed program, and when the surviving branch is
computed, produces a component that does not even validate.

**Root cause (const-fold in a function body drops the pair-of-branches check).** The seed const-folds a
constant-condition `if` to its taken branch. At the top level this fold still runs the branch-type
agreement check first; inside a function body it folds to the taken branch WITHOUT that check. The
internal checks of the dropped branch DO survive — an else `(+ 1 true)` is still rejected "operation on
mismatched types", an unbound else name is still CDZ0101 — so scope and internal-type checking of the
untaken branch happen; only the branch-type-AGREEMENT comparison (then-type vs else-type) is lost. When
both branches are constants the fold collapses to a literal and the mismatch is silently accepted
(`(def (f) (if true 1 false))` → 1); when the surviving branch is a non-const computed expression, a
real branch of the taken type is emitted while the dropped branch's incompatible representation was
never reconciled, so code generation emits an invalid component. This is exactly the failure the
top-level dead-branch cases warn about — "a fold that eliminates a branch must not eliminate its
type-check" — occurring on the in-function code path the top-level cases do not exercise.

**The lesson (a fold that drops a branch must run the agreement check FIRST, on every code path).** The
branch-type-agreement check must happen before (or independently of) the const-condition fold, and on
every path that fold runs — the top-level entry expression AND every function body. The seed runs it on
one and not the other, so the same `if` is rejected or accepted purely by whether it sits in `main`'s
own body or a called function. Master-pattern family: a check proven on one context (top-level) must
carry to the sibling context (inside a function). The tell: `(def (f) (if true 1 false))` runs but the
byte-identical top-level `(if true 1 false)` is rejected.

**Fix direction (gitignored seed).** Run the conditional branch-type-agreement check unconditionally at
type-check time, before the const-condition fold and regardless of whether the `if` is the entry
expression or nested in a function body. Equivalently: the fold that eliminates a branch must be
gated behind the same agreement check the top-level path performs — do not let a function-body fold path
bypass it. Regression guards: a top-level mismatched `if` still rejects (CDZ0201); a runtime-condition
mismatched `if` still rejects ("differ in kind"); a well-typed constant-condition `if` inside a function
still folds and runs (`(def (f) (if true 1 2))` → 1, `(def (f n) (if true (+ n 1) 9))` → n+1); the
internal-error checks of a dropped branch still fire.

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"a conditional inside a function
with a constant condition and mismatched branches is a type error" — `(def (f) (if true 1 false))` MUST
be CDZ0201. Placed right after the compound-dead-branch case, framed as the in-function companion of the
top-level dead-branch cases. Realized (functions + conditionals are realized), the behavior gate catches
it (expected CDZ0201, observed the program runs to 1; the computed-branch variant emits an invalid
component).

Related: the top-level dead-branch cases (`(if true 1 false)`, `(if false (record (a 1)) 7)`, tuple
mismatches — same file, the family this extends to the in-function context); the const-fold-drops-scope-
check case at l.211 (the scope-check analogue — "a const-fold to the taken branch must still scope-check
the other"); core-semantics.md #Conditionals Evaluate One Branch. This is the type-agreement analogue of
that scope-check case, on the in-function const-fold path. The recent learning "a fold that eliminates a
branch must not eliminate its type-check" is exactly this principle; this pins its in-function instance.
