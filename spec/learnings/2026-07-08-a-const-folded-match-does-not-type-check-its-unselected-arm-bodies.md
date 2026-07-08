# A const-folded match does not type-check its unselected arm bodies

*2026-07-08*

**What happened.** Adversarial probing of match arm-body type agreement found that a match on a
CONSTANT scrutinee does not type-check the arms it does not select. `(match 5 (5 1) (_ true))`
runs to `1` — the arms are `1` (Int64) and `true` (Bool), different types, so the match is
ill-typed, but the constant scrutinee `5` selects the Int64 arm and the compiler emits only that,
never checking the Bool arm. The same escape covers a 2-tuple/3-tuple arm mismatch and an
int/tuple arm mismatch. The identical arm-type disagreement is correctly rejected when the
scrutinee is a RUNTIME value ("runtime match arms differ in kind") and when the analogous check is
on a conditional ("`(if (= 5 5) 1 true)` → conditional branches have different types").

**Why it is a break.** A match is an expression of ONE type — all arm bodies must agree
(core-semantics.md #Matching Is Exhaustive Or Rejected makes a match's type what its arms yield;
02-binding-and-control.sexp §"a match on a runtime integer scrutinee producing a boolean" pins "a
match is an expression of whatever type its arms yield"). And #Conditionals Evaluate One Branch
requires "every branch … type-checked whether or not it is evaluated," so an unevaluated branch
cannot carry a deferred error — the same discipline applies to a match's unselected arms. Emitting
the folded arm while skipping the others' type-check is exactly the deferred-type-error the rule
forbids: an ill-typed program compiled to a running component.

**Root cause — const-folding the match fuses selection with emission, skipping the other arms.**
When the scrutinee is a compile-time constant, the compiler decides the matching arm at compile
time and emits only that arm's body (the same const-fold that makes `(match 5 (5 1) (_ 0))` emit
`1` directly). But it emits the selected arm WITHOUT first type-checking the whole arm set for
mutual agreement — so a disagreeing unselected arm is never seen. The runtime-scrutinee path emits
a real dispatch over all arms and therefore checks them (it reports "arms differ in kind"), and the
conditional path checks both branches before folding. The const-folded match is the one path that
folds-then-emits without the arm-agreement check. The fix is to run the arm-body-type-agreement
check on the full arm set BEFORE (or independently of) the const-fold that selects one arm —
exactly as the conditional's branch-agreement check runs independently of which branch a constant
condition selects.

**The lesson (same shape as the if-branch and dead-code findings).** A type-checking obligation over
a SET of alternatives — conditional branches, match arms — must be discharged over the whole set,
independent of any evaluation/const-fold that picks one. The recurring defect is a check fused with
emission (check-what-you-emit): const-folding emits only the selected alternative, so the check that
should cover all alternatives sees only the emitted one. The tell here was the three-way split for
the SAME arm disagreement: rejected under a conditional, rejected under a runtime-scrutinee match,
accepted under a const-scrutinee match — the const-fold is the hole. Checking is over all arms;
folding may then select.

**Corpus case added.** `spec/semantics/02-binding-and-control.sexp` §"a match whose arm bodies have
different types is a type error even when a constant scrutinee selects one" — `(match 5 (5 1) (_
true))` MUST reject CDZ0201, as the match analogue of the conditional branch-agreement cases and
the const-fold companion of the runtime-match arm-kind check. Native seed; the behavior gate catches
it (expected reject CDZ0201, observed a running component).
