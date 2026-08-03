# PR #1805 review comments — .github/workflows/checks.yml (v-nix) — OPEN

https://github.com/camshaft/cadenza/pull/1805 (wire nix flake check as an ADVISORY / non-required job).

## 1. "advisory / non-required" job still fails the workflow — missing continue-on-error (Copilot, checks.yml:396) — correctness/CI-config [VERIFIED]
> The job is labeled "advisory / non-required", but as written a `nix flake check` failure will still
> fail the job (and the whole workflow run). Mark `continue-on-error: true` so failures are reported
> without turning the workflow red.
VERIFIED against the diff: the job `name: nix flake check (advisory)` runs `nix flake check
--print-build-logs` (:41-42) with NO `continue-on-error: true`. So a flake-check failure fails the JOB →
reds the whole workflow, directly contradicting the "advisory / non-gating" intent (and the comment
that it's just for measuring wall-clock). Add `continue-on-error: true` (or set it non-required in branch
protection) so the advisory job reports without gating. MED — an "advisory" job that actually blocks would
red every candidate on a flake-check hiccup (and nix flake check is the deliberately-uncached slow one).
Fix BEFORE land. RECOMMEND v-nix confirm the intent (truly advisory → continue-on-error).

## 2. `@main` third-party action = supply-chain / stability risk (Copilot, checks.yml:400) — supply-chain
> Using `@main` for `DeterminateSystems/nix-installer-action` depends on a moving ref (supply-chain +
> stability risk). Pin to a major version tag or a full commit SHA.
The diff comment acknowledges "@main is the documented quickstart ref; pinned once confirmed" — so it's a
known temporary, but a moving @main on a third-party action that installs Nix is a real supply-chain
surface (aligns with the operator's supply-chain caution). Pin to a released tag/SHA before this lands as
a standing CI job, not "once confirmed". LOW-MED/supply-chain. Fix-forward.
