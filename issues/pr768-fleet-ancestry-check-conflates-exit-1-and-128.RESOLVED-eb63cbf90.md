# PR#768 review comment — batch-commit ancestry check conflates git exit 1 (not-ancestor) with 128 (fatal)

Mirrored from GitHub PR review comment (Copilot), id `3627695458`.
PR: https://github.com/camshaft/cadenza/pull/768 (batch-staging; fix belongs on trunk)
Location: `xtask/src/fleet.rs:6119` (the ancestry check added in `a763f5f2d`, the PR#765 safety fix)

## Comment (verbatim)

> `git merge-base --is-ancestor` has three relevant exit codes: 0 (ancestor), 1 (not ancestor), and
> 128 (fatal error, e.g. an invalid/stale SHA). The current `.status.success()` treats 1 and 128 the
> same and emits "NOT a descendant", which is misleading when the command actually failed. Capture the
> Output and branch on the exit code so fatal errors surface stderr (and keep the existing message for
> the true non-FF case).

## Liaison verification (CONFIRMED on trunk/staging)

fleet.rs:6110-6112:
```rust
let is_ff = git_out(&["merge-base", "--is-ancestor", &staged_base, &staged_tip])
    .status
    .success();
if !is_ff { return Err("... the staged tip ... is NOT a descendant ..."); }
```
`.status.success()` is true only on exit 0. Exit 1 (genuinely not-ancestor) AND exit 128 (fatal: an
invalid/missing/corrupt SHA, a broken repo) BOTH yield `is_ff = false` → the same "NOT a descendant …
staging ref is stale or corrupt" message. That message is CORRECT for exit 1 but MISLEADING for 128,
which hides the real git error (bad object, etc.) — exactly the diagnostic that matters when a staging
ref is corrupt.

This is a refinement of the good PR#765 safety fix (v-fleet-tooling `a763f5f2d`): the guard correctly
REFUSES in both cases (so it's still safe — trunk not moved), but the operator-facing diagnostic
conflates two distinct failures.

Fix (per Copilot): capture the full `Output`; branch on `status.code()` — `Some(0)` → FF ok;
`Some(1)` → the existing non-FF message; anything else (128/None) → a distinct "git merge-base failed:
<stderr>" error that surfaces the real fault. Owner: v-fleet-tooling (`fleet.rs` single owner). Routed
as a note.
