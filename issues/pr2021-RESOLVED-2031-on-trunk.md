# PR #2021 review — rcdzc effects.rs + tests.rs (v-effects) — MERGED — doc + test-coverage [VERIFIED, LOW] (batched)

https://github.com/camshaft/cadenza/pull/2021 (FIX branch/arm-body face of inner-abort-rolls-back-outer).
Copilot 3 inline: a stale comment + two test nits. All LOW.

## comment says "branch/SCRUTINEE face" + strict-operand "queued", but the helper is only called for `if` branches + match ARM bodies (not scrutinees), and strict-operand already landed with a test (Copilot, effects.rs:4428 & :4431) — doc-accuracy [VERIFIED]
> The new comment describes this helper as the "branch/scrutinee" face and says the strict-operand lift is
> "queued", but this function is only called for `if` branches and match ARM bodies (not scrutinees), and
> strict-operand already has a dedicated regression test in this module. Please update the comment…

VERIFIED on trunk (effects.rs:4426): comment reads "the branch/scrutinee face of the abort-outer-advance
fix; the direct-handle-body do-shape landed #2002, the strict-operand lift is queued". TWO inaccuracies: (a)
"scrutinee" — the helper fires on branch bodies (`if` then/else + match arm bodies), NOT the
scrutinee/condition (a perform there is on the strict spine, threaded normally — the other findings in this
arc say exactly that); (b) "strict-operand … queued" — strict-operand landed in #2010 (with a regression
test). Fix: reword to "branch/arm-body face" and drop/update the "queued" clause (strict-operand landed
#2010). LOW/doc.

## test name says "in a branch" but exercises specifically an `if` branch (Copilot, tests.rs:66533) — test-naming [VERIFIED]
> Renaming it to match the existing `...in_a_match_scrutinee...` / `...in_a_strict_operand...` pattern will
> make it easier to find and cross-reference with the semantics case name.
VERIFIED — rename to `..._in_an_if_branch` for consistency with the sibling test names. LOW.

## the fix covers BOTH `if` branches AND match ARM bodies, but the new Rust test only exercises the `if` branch call site (Copilot, tests.rs:66537) — test-coverage [VERIFIED]
> Consider adding a second unit test that uses `(match 0 (_ (do (A.tick) (B.bail 99))))` so both call
> sites are covered at the compiler-test level too.
VERIFIED — the PR adds a semantics corpus case for the match-arm-body face + code covering both call
sites, but the compiler-level regression test only drives the `if` branch. Add a match-arm unit test
(`(match 0 (_ (do (A.tick) (B.bail 99))))`) so a regression in the match-arm path is caught at the
compiler-test level too (not only via the corpus). LOW/test-coverage — the corpus case covers behavior, but
a targeted unit test localizes a regression faster. v-effects owns effects.rs + rcdzc/tests.rs. All 3 LOW,
batchable.
