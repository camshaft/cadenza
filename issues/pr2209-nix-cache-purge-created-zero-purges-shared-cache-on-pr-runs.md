# PR #2209 review — .github/workflows/checks.yml (v-nix) — OPEN — 1 CI-reliability (MED) + 1 LOW [VERIFIED]

https://github.com/camshaft/cadenza/pull/2209 (PILOT the /nix/store cache on the rustfmt job —
cache-nix-action, save-on-trunk). Copilot 2 inline.

## `purge: true` + `purge-created: 0` (+ prefix purge) makes EVERY run — including PR/candidate runs — eligible to delete all caches matching the prefix → can purge the shared trunk cache candidates are meant to RESTORE from, thrashing it (Copilot, checks.yml:57) — CI-reliability [VERIFIED, MED]
> `purge: true` together with `purge-created: 0` will make *every* run eligible to delete all caches
> matching the `purge-prefixes`, including PR runs. In cache-nix-action, `purge-created` is in seconds and
> `0` effectively disables the age threshold, which can thrash the shared cache and cause restore
> misses/rate-limit churn.

VERIFIED in the #2209 diff: `purge: true` (diff:21), `purge-prefixes: nix-${os}-${arch}-` (diff:22),
`purge-created: 0` (diff:23), `purge-primary-key: never` (diff:24). `save` is gated to trunk-only (diff:19,
`github.ref == default_branch`). So: candidates RESTORE the trunk cache but don't SAVE — good. BUT `purge:
true` runs on EVERY invocation (no branch gate on purge), and `purge-created: 0` = no age floor, so a PR/
candidate run is eligible to PURGE all caches matching `nix-${os}-${arch}-` EXCEPT its own primary key
(protected by `purge-primary-key: never`). Since the trunk cache's key differs from a candidate's
`hashFiles('flake.lock')` key only if the lock differs — but the PREFIX matches — a candidate can purge the
very trunk cache it's supposed to restore from → restore-misses + re-download churn, partially defeating the
pilot's stated goal ("candidate runs RESTORE a shared base without each writing"). MED/CI-reliability. Fix
per Copilot: gate `purge` to trunk-only too (same `github.ref` expression as `save`), OR set a sane
`purge-created` age threshold so candidates don't purge fresh shared caches, OR `purge-prefixes` scoped so
it can't hit the trunk base. v-nix owns CI. (This is a PILOT on one job before rolling to 7 others — worth
getting the purge scope right BEFORE the rollout amplifies it.)

## the comment says "trunk (default branch)" but GHA's default branch is `github.event.repository.default_branch` (main here) — "trunk" misleads a CI-debugging reader (Copilot, checks.yml:45 & :50) — doc [VERIFIED, LOW]
> The comment refers to "trunk (default branch)", but in GitHub Actions the default branch is
> `github.event.repository.default_branch` (typically `main` here). Using "trunk" in the workflow comment
> is misleading for readers debugging CI behavior.
VERIFIED (diff:10 "trunk (default branch)"). The fleet's internal integration branch is called "trunk", but
GHA + the repo's actual default branch is `main` — a CI reader debugging `github.ref` sees `main`, not
`trunk`. LOW/doc. Fix: say "the default branch (main)" in the comment, or note trunk==the fleet's name for
main. v-nix owns CI. PR OPEN → both foldable. The purge scope is the one that matters (thrash risk before a
7-job rollout).
