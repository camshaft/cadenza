# PR #1690 review comment — xtask/src/fleet.rs (v-fleet-tooling) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1690 (MERGED). Follow-on to #1684's batched-git-show.

## `git_stdout` trims whole output → empty commit subject drops a line → positional zip misaligns → wrong commit dropped (Copilot, fleet.rs:5444) — correctness [VERIFIED]
> `git_stdout` trims the ENTIRE stdout, dropping a leading/trailing empty subject line (a commit with an
> empty subject). Because the code relies on positional zipping (`subjects[i]` ↔ `replay[i]`), global
> trimming can misalign indices and drop the WRONG commit. Parse the raw output without a global trim;
> `split_terminator('\n')` to preserve empty subjects mid/end while avoiding the final-newline artifact.

VERIFIED: `git_stdout` (fleet.rs:5308) does `String::from_utf8_lossy(&git(args).stdout).trim()
.to_string()`. The #1684 batched filter does `git_stdout(&["show","-s","--format=%s", <sha…>]).lines()
.collect()` then filters `replay` by positional `subjects.get(i)`. A commit with an EMPTY subject
(`git commit --allow-empty-message`, or some merge/import commits) yields an empty line; `.trim()` strips
a leading/trailing one and the `.lines()` count drops below `replay.len()` → `subjects.get(i)`
misaligns → the archive-mirror filter drops the WRONG commit (or misses the mirror). Real latent
index-misalignment introduced by the #1684 batching. Fix per Copilot: drop the global `.trim()` for this
call (or use a non-trimming read) + `split_terminator('\n')`. MED (narrow — needs an empty-subject commit
in the replay window — but a silent wrong-commit-drop on the sync path). Fix-forward.
