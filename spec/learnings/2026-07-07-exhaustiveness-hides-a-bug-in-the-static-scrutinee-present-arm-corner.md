# Exhaustiveness hides a bug in the static-scrutinee, present-arm corner — the check must key on the arm set, not the value the scrutinee holds

*2026-07-07*

**What happened.** While confirming the reference seed is a faithful ask-30 rejection oracle, the compiler agent
hit an asymmetry in bool-match exhaustiveness: `(match true (false 0))` correctly rejected CDZ0210 ("does not
cover the scrutinee"), but `(match true (true 1))` compiled VALID — a non-exhaustive match that escaped rejection.
The static-scrutinee match path (where the scrutinee is a compile-time constant) checked only SUM exhaustiveness,
then returned the arm the constant `true` matched, never verifying the arm set covers `false`. The fix: check bool
exhaustiveness in the static path too, keyed on the arm set versus the type — not on which value the constant
holds.

The gap it exposed in the corpus: the two existing bool-exhaustiveness cases both used a PARAMETER scrutinee
(`(match b (true 1))`, `b` a function param — the dynamic path), and the present-arm-hit case existed only for
SUMS (`(match (Some 5) ((Some x) x))`). There was no case for a CONSTANT bool scrutinee that hits its sole present
arm — exactly the static path where the bug lived. I added it: `(match true (true 1))` → CDZ0210 (behavior gate
573 → 574).

**Why.** Exhaustiveness has a two-by-two of compile paths that must each be tested, and the bug always hides in
the same corner. The axes are (a) does the sole arm name the value the scrutinee holds, or a different value —
"present-arm" vs "missing-value"; and (b) is the scrutinee a compile-time CONSTANT (static path) or a runtime
PARAMETER/expression (dynamic path). The **missing-value** forms are easy to get right, because the check trivially
notices the arm doesn't match and has to do *something*. The **present-arm** forms are where correctness leaks,
because a natural implementation does "find the arm that matches, return it" and a present arm short-circuits that
search BEFORE any exhaustiveness check runs — the coverage question never gets asked. And the **static** path is
where it leaks worst, because a constant scrutinee invites exactly that shortcut ("I know the value, I know which
arm, done"), while the dynamic path is forced through general match compilation that already had the
exhaustiveness check wired. So the dangerous cell is static × present-arm: constant scrutinee, sole arm names the
constant's value — and that is precisely the cell `(match true (true 1))` occupies and the one the corpus lacked.

The load-bearing principle, stated so it generalizes: **exhaustiveness is a property of the ARM SET against the
TYPE's value set, never of the value the scrutinee holds.** Any implementation that consults the scrutinee's value
to decide whether to check coverage has the bug latent — it will pass every missing-value test and every dynamic
test and still mis-accept the static present-arm form. The corpus must therefore test the CROSS PRODUCT, not a
diagonal: both bool values × both (present-arm, missing-value) × both (constant, parameter) scrutinees. The
existing cases covered parameter × {present, missing} and sum × constant × present; the hole was bool × constant ×
present, and a bug sat in exactly the hole. A test suite that covers "bool exhaustiveness" with only
parameter-scrutinee cases is testing one path and claiming the property.

**The requirement it drove.** Corpus: "a bool match on a constant scrutinee is non-exhaustive even when the
constant hits the sole arm" — `(match true (true 1))` → CDZ0210, the constant-scrutinee present-arm form, the
companion of the existing parameter-scrutinee cases and the constant-sum present-arm case (behavior gate 573 →
574). Its job is to guard the static-path exhaustiveness check that a present-arm constant scrutinee would
otherwise let skip. General lesson: **exhaustiveness must key on the arm set versus the type, not on the
scrutinee's value; the bug hides in the static-scrutinee, present-arm corner (a constant whose sole arm names its
own value invites a shortcut that skips the coverage check), so a corpus proving "exhaustiveness works" must test
the cross product of {present-arm, missing-value} × {constant, parameter} scrutinees, not just the easy diagonal.**
