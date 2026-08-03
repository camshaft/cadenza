# PR #1077 review comments — fleet/slack-bridge/src/sidecar.rs (v-slack-bridge)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1077
(PR: "cand: v-slack-bridge — sidecar.rs"). This is the follow-up to PR #1044's prune review; the
new comments are about the complexity CLAIMS in comments not matching the implementation.

## 1. O(n) comment vs BTreeSet::contains cost (Copilot, sidecar.rs:110, also :116) — doc
> The complexity note says pruning is O(n) over `by_key`, but the current implementation uses
> `BTreeSet::contains`, which makes the retain passes O(n·log k) (and building the set is
> O(k·log k)). Either adjust the comment, or use a hash set for membership so the claim is
> (amortized) accurate.

## 2. Test comment overclaims it "pins" O(n) (Copilot, sidecar.rs:385) — doc/test
> This test comment says it "pins" the O(n) complexity, but the test only verifies functional
> behavior (which is great) and can't actually assert big-O. Consider rewriting the comment to
> describe the behavioral contract being tested.

Both are comment/wording accuracy points on the prune rework — either make the comment match
(O(n·log k)) or switch to a HashSet for the amortized-O(n) claim to hold.
