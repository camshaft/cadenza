# PR#796 review comment — sread-eval-nonrec.cdz header references a gitignored .claude/ queue file (dangling pointer)

Mirrored from GitHub PR review comment (Copilot), id `3632909302`.
PR: https://github.com/camshaft/cadenza/pull/796 (batch-staging; fix belongs on trunk)
Location: `implementation/compiler-ml/src/sread-eval-nonrec.cdz:5`

## Comment (verbatim)

> The header comment references `queue/vcml-probe-2026-07-22-…md`, but that file is not part of the
> repository (and `.claude/` queue artifacts are typically gitignored). This makes the pointer unusable
> for future readers; consider removing the reference or replacing it with a committed link (issue/PR)
> that will persist.

## Liaison verification (CONFIRMED on trunk)

sread-eval-nonrec.cdz:1-3 header: "…surfaced green by the 2026-07-22 run-ml probe (see
queue/vcml-probe-2026-07-22-nonrecursive-shapes-all-green-candidate-regression-tests.md)." The fleet
`queue/` lives under `.claude/` (gitignored, hub-only) — it is NOT committed to the repo, so a reader
of the tracked source can't follow the pointer. Dangling doc reference.

Fix (per Copilot): drop the reference or replace it with a persistent committed link (the PR that added
the file, or an inline one-line summary of what the probe found). Doc-only.

Owner: v-compiler-ml (`implementation/compiler-ml/*` port source — this is the sibling regression file
they added). Routed as a note. Minor.
