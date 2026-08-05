# PR #2313 review — flake.nix + .github/workflows/checks.yml (v-nix) — OPEN — 2 doc/naming-accuracy [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2313 (scope the nix-flake advisory to a reproducibility-backstop
subset — cut the 48.6m redundant whole-flake rebuild; the v-nix+v-ft CI-speed work). Copilot 2 inline.

## c1 — comment says reducer-cadenza "b1-b4/genesis" but the flake only defines B1/B2/B3 + genesis (no B4) (Copilot, flake.nix:1615, also 1655) — doc-accuracy [VERIFIED-weak, LOW]
> Comment mentions reducer-cadenza "b1-b4/genesis", but this flake only defines reducerCadenzaB1/B2/B3 and
> reducerCadenzaGenesis (no B4). ... risks future confusion. This issue also appears on line 1655.
PER-SITE VERIFIED (per [[liaison-copilot-also-appears-at-line-N-secondary-occurrence-needs-per-site-verify]]):
- 1615 IS a "b1-b4/genesis" mention. REAL.
- the "also at 1655" is a FALSE secondary: flake.nix:1655 is `reducer-cadenza-tests = reducerCadenzaTests;`
  — no "b1-b4" wording. Copilot mis-matched.
- IMPORTANT context: "B1-B4" is an ESTABLISHED project label for the bootstrap arc, used PRE-#2313 at
  flake.nix:576 ("All B1-B4 reducers export") and :1516 ("the B1-B4 reducer wasm COMPONENTS") — B4 is a
  planned/known member of the series; only B1/B2/B3 have COMPONENTS wired today. So "b1-b4" describes the
  arc's SCOPE, not a claim that 4 components exist. WEAK — arguably a pre-existing convention, not a
  #2313-introduced inaccuracy. If v-nix wants precision it can say "b1-b3/genesis (b4 pending)"; optional.

## c2 — the job is still named "nix flake check (advisory)" but no longer runs `nix flake check` — it runs the scoped `flake-repro-backstop` build; rename for Actions-UI / branch-protection clarity (Copilot, checks.yml:480) — naming-accuracy [VERIFIED, LOW]
> The job still advertises itself as "nix flake check (advisory)", but it no longer runs `nix flake check` —
> it runs the scoped reproducibility backstop build. Renaming the job helps keep the Actions UI and
> branch-protection context clear.
VERIFIED: the job `name: nix flake check (advisory)` (checks.yml ~460) while the STEP was renamed to "nix
reproducibility backstop (scoped from full flake check)" and runs `nix build .#checks…flake-repro-backstop`.
So the step is accurate but the JOB name still says "flake check." LEGITIMATE accuracy point. NUANCE for
v-nix: the job `name:` IS the branch-protection CONTEXT string (`checks / nix flake check (advisory)`).
Renaming a REQUIRED context needs a ruleset edit — but this job is ADVISORY (continue-on-error, explicitly
non-required per the diff), so a rename is likely safe. Still v-nix's call whether to rename given the fleet's
usual context-string caution. Fix: rename the job to match (e.g. "nix repro backstop (advisory)"), confirming
no required-context depends on the old name.

v-nix owns flake.nix + checks.yml. PR OPEN → both foldable. c1 is weak/optional (B1-B4 is an existing arc
label); c2 is a fair naming-accuracy nit with a context-string caveat.
