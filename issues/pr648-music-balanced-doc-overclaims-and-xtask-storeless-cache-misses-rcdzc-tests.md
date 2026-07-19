# pr648 — music `balanced` doc overclaims the invariant + xtask storeless-cache misses rcdzc test edits (2 Copilot)

Mirrored from GitHub PR #648 review comments (Copilot). Both VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/648 (7-MR batch + run_ml CI-fix)

## #1 — id 3611307448 (implementation/music/src/schedule.cdz:14) — balanced doc oversells [v-music]
> The header comment says `balanced` checks that per-(channel,note) outstanding note-ons "never goes negative
> and returns to zero", but the implementation only checks `net_total(evs, 0) == 0` (global net) and doesn't
> enforce any non-negative prefix condition or per-key balancing. This makes the docs misleading for callers
> who rely on the stronger invariant.

VERIFIED: header (schedule.cdz:11-13) defines BALANCED as "the count of outstanding note-ons PER (channel,
note) never goes negative AND returns to zero at the end" and says "`balanced` checks exactly that." But
`def balanced(evs) = net-total(evs, 0) == 0` (line 146), and `net-total` (143) is a GLOBAL on/off counter —
no per-(channel,note) split (there IS a `net-outstanding` per-key helper at 127, but `balanced` doesn't use
it) and no non-negative-prefix check. So `[off, on]` (global net 0 but goes negative) or per-key imbalances
that cancel globally would PASS `balanced` despite violating the stated stuck-key invariant. Either strengthen
`balanced` to the documented invariant (non-negative prefix + per-key), or weaken the doc to "checks global
net returns to zero." Since the doc calls this THE stuck-key correctness backbone, strengthening is likely
right. → v-music (new music vertical).

## #2 — id 3611307439 (xtask/src/main.rs:3246) — storeless-cache key misses rcdzc --lib test edits [v-fleet-tooling]
> The storeless-rerun cache key currently assumes "rcdzc `--lib` tests are compiled into the `cdz` binary",
> but rcdzc's unit tests live under `#[cfg(test)]` (rcdzc/src/lib.rs:136-140), so changing rcdzc test sources
> will *not* change the `cdz` binary. As written, the cache can incorrectly skip the storeless rerun after
> edits to rcdzc's test sources, reducing coverage and reintroducing local-green/CI-red surprises.

VERIFIED: the `storeless-rerun` CachedStep keys on `(Path::new(&cdz) ‖ paths.seed.join("crates/cdz/tests"))`
(main.rs:3243-3247). rcdzc `#[cfg(test)]` unit tests are NOT compiled into the release `cdz` binary and are
NOT under `cdz/tests`, so editing them flips NEITHER cache input → the ~230s storeless rerun is skipped even
though the rcdzc guarded-test set changed. NB mitigant: CI's `test` job runs `cargo test --workspace`
separately (broader than this cached step — see my own note [[ci-test-job-runs-cargo-test-workspace-not-just-rcdzc-lib]]),
so rcdzc lib tests aren't ONLY gated here — but the storeless-rerun cache's own coverage claim is still
inaccurate. Fix = add the rcdzc src/test tree (or the rcdzc test sources) to the cache key. → v-fleet-tooling
(owns xtask).

## Owner
#1 v-music, #2 v-fleet-tooling. Split.
