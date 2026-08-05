# PR #2234 review — .github/workflows/checks.yml (v-nix) — OPEN — CI-permissions [PLAUSIBLE-MED, check the merged pilot]

https://github.com/camshaft/cadenza/pull/2234 (roll out the /nix/store cache to all 8 nix jobs via a
composite action). Copilot 1 inline — a CI-breaking-permissions claim. Relaying CALIBRATED (per my #2209
lesson: a claim hinging on a third-party action's runtime perms is PLAUSIBLE, not certain — I can't verify
whether cache-nix-action REQUIRES `actions:write` vs degrades gracefully).

## Copilot: the cache step needs `actions: read` (restore) + `actions: write` (save/purge), but the caller `ci.yml` sets `permissions: { contents: read }` (no `actions`), so the cache step may fail across all PR/main runs once it's in all 8 jobs (Copilot, checks.yml:42) — CI-permissions [PLAUSIBLE-MED]
> The new nix-store cache step will likely fail (and can stop the job before running `nix build`) because
> the workflows that call this reusable `checks.yml` currently scope `GITHUB_TOKEN` permissions without
> granting `actions:*`. For example, `.github/workflows/ci.yml` sets `permissions: { contents: read }`,
> which removes `actions` permissions entirely; cache restore/save/purge typically requires `actions:
> read` (restore) and `actions: write` (save/purge). With the cache step now present in multiple jobs,
> this can break all PR/main CI runs unless the caller workflows grant the needed permissions.

VERIFIED (the parts I can, from source): `ci.yml:24-25` sets `permissions: { contents: read }` and calls
`checks.yml` (:29); `checks.yml` has NO `permissions:` block of its own (so it inherits the caller's
`contents: read`); #2234 adds NO permissions block either. Other callers: `nightly.yml`, `release.yml`.
So the factual premise — the cache step runs under `contents: read` with no `actions` grant — is CONFIRMED.

WHAT I CAN'T VERIFY (owner's call): whether `cache-nix-action` actually REQUIRES `actions: read`/`write`
(GHA cache-API perms), or degrades gracefully (cache-miss / skip save) when `actions` isn't granted. That's
the action's runtime behavior + GHA token semantics — not repo source. So Copilot's "will likely fail /
break all CI" is PLAUSIBLE-MED, NOT certain (I over-asserted exactly this class on #2209 — a third-party
action's runtime behavior).

DISPOSITIVE EMPIRICAL CHECK (please use, v-nix): the rustfmt-only PILOT is ALREADY ON TRUNK (checks.yml:47,
merged #2214/#2231) running THIS cache-nix-action under THIS same `contents: read`. So its CI runs already
answer the question:
- If the pilot's cache step has been RESTORING/SAVING green on trunk under `contents: read` → the perms are
  already sufficient (GHA may auto-grant `actions` read, or the action no-ops gracefully) → Copilot's
  finding is a FALSE ALARM and the 8-job rollout is safe. Dismiss-on-green.
- If the pilot's cache step has been silently FAILING or warning (cache always-miss, or a step error that
  didn't fail the job) → Copilot is RIGHT, and the rollout amplifies it to all 8 jobs → add `actions: read`
  + `actions: write` to the caller (ci.yml/nightly/release job-level on the `uses: checks.yml` job, or
  checks.yml workflow-level) BEFORE the 8-job rollout lands.
Either way the merged pilot's run logs settle it definitively — you own the action + CI, so you can read
those directly. If it needs perms, fixing it now (before #2234 lands) prevents the rollout breaking all CI.
v-nix owns CI. PR OPEN → resolve before the 8-job rollout merges. (Calibration: PLAUSIBLE-MED, not asserted
MED — the perms-gap is real in source, but whether cache-nix-action FAILS on it is the runtime question the
pilot logs answer.)
