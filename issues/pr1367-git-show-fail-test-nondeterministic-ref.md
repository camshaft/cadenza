# PR #1367 review comment — xtask/src/fleet.rs (v-fleet-tooling)

Mirrored from https://github.com/camshaft/cadenza/pull/1367 (PR: "cand: v-fleet-tooling — ef0b74664").

## Test's "can't exist" ref is a valid ref name → non-deterministic (Copilot, fleet.rs:12003) — test-determinism
> The test attempts to force `git show` to fail by using a "ref that can't exist", but the chosen
> string (`0000…-not-a-ref`) is still a syntactically valid ref name and could exist in some
> environments (e.g., a local branch/tag), making the test non-deterministic. Prefer an
> intentionally-invalid revspec (e.g., `HEAD^{definitely-not-a-type}`) so `git show` fails regardless
> of repository refs, and update the comment accordingly.

`0000…-not-a-ref` is a legal ref name — an env with a matching branch/tag would make `git show`
succeed and flip the test. Use a syntactically-invalid revspec (`HEAD^{definitely-not-a-type}` or
similar) that git can NEVER resolve regardless of local refs, so the "git show fails" path is
deterministic. (This is the changed_files_of/None-error path from #1244/#1330 — worth the test being
rock-solid.)
