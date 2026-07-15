# PR review comment — mirrored from GitHub PR #375 (Copilot inline)

- **PR:** #375 "fleet: integrate the second batch (module resolve, borrowed-key fix, wasm if-hoist, roles)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/lower.rs:13433`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3589185980
- **Link:** https://github.com/camshaft/cadenza/pull/375#discussion_r3589185980

## Comment (verbatim)
> In the hoisted form, `cond` is evaluated inside synthesized per-payload `if`s, so it can be evaluated *after* any shared payloads (depending on tuple/sum payload order). If `cond` can trap, this can change which trap is observed vs the original `if` semantics (where `cond` is evaluated before any arm payloads). Guard the transform so a potentially-trapping `cond` is only allowed when the first payload is the differing one (evaluated first), otherwise require `cond` to be trap-free.

## Liaison triage
Potential CORRECTNESS bug in the common-constructor if-hoist: a trapping `cond` could change which
trap is observed (eval-order semantics). This is exactly the class of "defined trap gets reordered"
miscompile the fleet already tracks. Route to `corpus-bugfix` PM for a fix agent to confirm against a
reproducer and guard the transform. Fix belongs on `trunk` (PR merged).
