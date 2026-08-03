# PR #1358 review comment — cdz-agent-host/src/host.rs (v-agent-harness-host)

Mirrored from https://github.com/camshaft/cadenza/pull/1358 (PR: "cand: v-agent-harness-host — 2eb99a6a7").
Follow-through on the #1344 fork_query -> fork_for_query rename.

## Leftover "Fork-query" test comment after the rename (Copilot, host.rs:517) — doc
> This test comment still refers to "Fork-query" even though the API and terminology were renamed to
> "fork-for-query" (and the test/function names were updated). Updating the comment avoids confusion
> when searching for the feature name.

The #1344 rename updated the API + test/function names but left this comment saying "Fork-query" —
update it to "fork-for-query" so a search for the feature name finds it.
