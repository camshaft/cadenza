# A const-scrutinee match fold swallows the unselected arms' scope check

*2026-07-08*

**What happened.** A `match` with a compile-time-constant scrutinee is const-folded to its selected
arm, and the fold scope-checks ONLY that arm — so an unbound name in an UNSELECTED arm is silently
swallowed rather than rejected. `(match 2 (1 undefined-z) (_ 99))` selects the `_` arm (scrutinee 2 ≠
1) and runs to **99**, though the `1` arm references the unbound `undefined-z` and the program MUST be
rejected CDZ0101. This holds at the top level and inside a function alike — it is not
function-specific.

**Why it is a break.** core-semantics.md #Binding Is Lexical: "A reference to a name with no enclosing
binding MUST be a compile-time error" — unconditional, and the corpus notes scope resolution "needs no
static typing," so every generation catches it. #Conditionals Evaluate One Branch requires every branch
type-checked whether or not evaluated; the corpus already extends this to a match's arms for TYPE
errors (the unselected-arm agreement case `(match 5 (5 1) (_ true))` → CDZ0201, and the internally-
ill-typed unselected-arm case `(match 5 (5 1) (_ (+ 1 true)))` → CDZ0201). An unevaluated arm cannot
carry a deferred SCOPE error any more than a deferred type error. The `if` form already enforces this
for its unselected branch: `(if true 1 undefined-name)` → CDZ0101 (a realized, passing case). The match
analogue is what the seed drops.

**Root cause (const-match fold scope-checks only the selected arm).** The seed const-folds a
constant-scrutinee match to the arm the scrutinee selects and emits only that arm; the unselected arms
are discarded WITHOUT scope resolution. So an unbound name in a dropped arm never reaches the CDZ0101
check. The `if` fold scope-checks its dropped branch correctly (verified: `(if true 1 undefined-z)` →
CDZ0101 at top level and in a function), so this is specifically the const-folded MATCH path. It is
strictly more permissive than the `if` fold: the match fold also drops the unselected arms' internal
TYPE check inside a function (`(def (f) (match 5 (5 1) (_ (+ 1 true))))` runs to 1), whereas the `if`
fold keeps internal type-checking of its dropped branch. The scope-check drop is the sharpest
manifestation — it swallows CDZ0101, the most basic front-end check.

**The lesson (a fold that drops an arm must scope-check it first, like the `if` fold does).** Scope
resolution — and type-checking — of a match's arms must happen before (or independently of) the
const-scrutinee fold, on every arm, exactly as the corpus requires for an `if`'s branches and already
pins for a match's arm TYPES. The seed carried the type-check to the const-folded match but not the
scope-check, and the `if`-branch scope rule was never carried to the match arm at all. Master-pattern
family: a check proven on one form (`if` unselected branch → scope-checked) must carry to the sibling
form (`match` unselected arm), and a fold that eliminates an arm must not eliminate any of its checks
(scope, internal type, agreement).

**Fix direction (gitignored seed).** Scope-check (and type-check) every arm of a match before folding a
constant scrutinee to its selected arm — do not let the fold path discard the unselected arms without
resolution. Equivalently: run the front-end scope pass over all arms unconditionally, then fold.
Regression guards: an unbound name in the selected arm still rejects (`(match 5 (5 undefined-z) (_ 1))`
→ CDZ0101); a well-typed const-scrutinee match still folds and runs (`(match 2 (1 10) (2 20) (_ 0))`);
the `if` unselected-branch scope case still rejects; the unselected-arm TYPE cases still reject.

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"an unbound name in an unselected
match arm is still rejected" — `(match 2 (1 undefined-z) (_ 99))` MUST be CDZ0101. Placed right after
the internally-ill-typed-unselected-arm case (its type-check companion), framed as the match companion
of the `if` unselected-branch scope case. Realized (const-scrutinee match is realized), the behavior
gate catches it (expected CDZ0101, observed the program runs to 99).

Related: the `if` unselected-branch scope case (`(if true 1 undefined-name)` → CDZ0101, same file — the
form this carries to match); the unselected-arm TYPE cases (`(match 5 (5 1) (_ true))`, `(match 5 (5 1)
(_ (+ 1 true)))` — the type-check companions this is the scope analogue of);
[[in-function-const-if-fold-drops-branch-type-check-break]] (c85 — the `if` in-function fold-drops-check
sibling); core-semantics.md #Binding Is Lexical, #Conditionals Evaluate One Branch. Broader observation:
the const-fold-drops-a-check family now spans if-in-function (c85) and match-everywhere (this) — a fold
that eliminates a branch/arm must run all of that branch/arm's checks first.
