# PR #1204 review comments — cdz-agent-host/src/model.rs (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1204 (PR: "cand: v-agent-harness-host — d71aeeabf").

## 1. Forward-looking "not supported yet / later slice" comments go stale (Copilot, model.rs:63, also :79) — doc
> These comments use forward-looking phrasing ("not supported yet" / "later slice"). To avoid stale
> docs, describe the current limitation and why blob payloads are rejected (this executor has no
> blob-store access), without implying a timeline.

## 2. Test name `..._for_now` has a temporal qualifier (Copilot, model.rs:142) — test naming
> Test name includes a temporal qualifier ("for_now"), which tends to become stale and doesn't
> describe the invariant being tested. Consider renaming it to state the behavior directly (blob
> payloads are currently rejected).

Both are the same "don't bake a timeline into docs/test-names" point: state the invariant (blob
payloads are rejected — no blob-store access in this executor) rather than "not yet / for now".
