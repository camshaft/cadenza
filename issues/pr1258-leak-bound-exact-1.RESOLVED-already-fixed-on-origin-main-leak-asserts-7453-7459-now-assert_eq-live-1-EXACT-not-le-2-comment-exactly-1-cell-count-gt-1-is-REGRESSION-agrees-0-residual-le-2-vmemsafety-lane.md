# PR #1258 review comment — rcdzc/src/tests.rs (v-memory-safety)

Mirrored from https://github.com/camshaft/cadenza/pull/1258 (PR: "cand: v-memory-safety — f383e234b").
This is the follow-up landing the #1219 leak-bound tighten — Copilot flags a residual over-correction.

## Comment says "measured EXACTLY 1" but assertion allows `live == 2` (Copilot, tests.rs:5206, also :5256) — test-tightness
> The comment says the leak count is deterministic and "measured EXACTLY 1", but the assertion allows
> `live == 2`, so a regression from 1→2 would not fail (and the message/comment about tightness
> becomes inaccurate). If this really is deterministic, prefer asserting the exact count (and adjust
> the failure message accordingly).

Follow-through on #1219: you confirmed the count is deterministically 1 and tightened `<= 16` → `<= 2`
(the margin I relayed from amazon-q). But since it's exactly-1-deterministic, `<= 2` still lets a 1→2
regression pass while the comment claims "EXACTLY 1" tightness. Assert `live == 1` (or keep `<= 2` but
soften the comment to admit the 1-cell margin) so the assertion and the "exactly 1" claim agree. Since
you measured it deterministic, `== 1` is the stronger, self-consistent choice.
