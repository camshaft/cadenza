# PR #1786 review comments — flake.nix (v-nix) — OPEN

https://github.com/camshaft/cadenza/pull/1786 (S3 — run a project's tests through nix).

## 1. `cdz test | tee` without pipefail HIDES test failures → red suite makes a GREEN derivation (Copilot, flake.nix:188) — correctness/CI-integrity [VERIFIED]
> `cdz test | tee ...` hides `cdz test`'s exit status: without `pipefail` the pipeline returns `tee`'s
> status, so failing tests can still produce a successful derivation — contradicting the comment that a
> non-zero exit fails the build. Enable `pipefail`.
VERIFIED against the diff: the build phase comment says "a failing `cdz test` exits non-zero → build
fails" (~:42), but the command is `cdz test | tee "$TMPDIR/test.out"` (:54) with no `set -o pipefail`. The
pipeline's exit status is `tee`'s (0), so a RED suite yields a GREEN derivation — the derivation
contradicts its own comment and the nix-test gate would pass broken tests. MED (defeats the whole point of
running tests through nix). Fix: `set -euo pipefail` (or `set -o pipefail`) before the pipe. Same
pipefail-discipline class as the #1572 componentStore note. Fix-forward/before-land.

## 2. install phase copies into `$out` without a stable output path (Copilot, flake.nix:196) — hygiene/determinism
> The install phase copies into `$out` without ensuring a stable output path.
Ensure the install writes to a deterministic layout under `$out` (matters for the nix-store determinism
north-star). LOW. Fix-forward.
