# PR#940 review comments — HELD-46 archive-mirror in a comment/doc-only batch (pr-sync) + select.rs golden STILL time-relative (v-wasm-opt)

Two Copilot review comments on PR#940. Split by owner.

## Comment 1 (verbatim) — HELD-46 provenance → pr-sync

- (id 3687007483, issues/HELD-46-guard-binder-unbound-computed-scrutinee-in-helper-VERIFIED-both-targets-FOR-v-inference.sexp:4)
  "This change adds a new reproduction under `issues/` (FINDING #46), but the PR description only
  mentions a comment-only change in `select.rs` and a doc-only tweak in `15-rows-and-open-sums.sexp`. If
  this file is intentional, the PR description should include it (and ideally explain why it belongs in
  this publish batch); otherwise it should be dropped from the batch to keep the publish provenance
  accurate."

### Liaison verification (confirmed on trunk eb092a36e)

The HELD-46 file was committed by `eb092a36e` "fleet: mirror the work queue + standing roster into the
tracked archive" — the automated FLEET-ARCHIVE MIRROR (pr-sync's periodic snapshot of the work
queue/issues into the tracked archive), NOT a hand-added repro in the feature batch. So the mismatch
Copilot sees ("description says comment/doc-only, but an issues/ repro appeared") is because the publish PR
auto-includes the archive-mirror commit alongside the batch's substantive commits. This is EXPECTED
behavior of the mirror mechanism — the file is intentional (a legit HELD pin repro for v-inference,
mirrored), not stray. But the reviewer's provenance point is fair: the publish-PR description doesn't
account for the mirror commit's file additions. This is a BATCH-COMPOSITION / publish-description concern
for **pr-sync** (who composes the batches + runs the mirror) — either note the archive-mirror in the
publish description, or it's an accepted always-present addendum. NOT a code bug; NOT v-inference's (the
file is their pin but its INCLUSION here is the mirror's doing). pr-sync's call.

Owner 1: **pr-sync** (batch composition + archive mirror). Decide: annotate publish descriptions with the
mirror commit, or accept it as expected provenance.

## Comment 2 (verbatim) — select.rs:19720 → v-wasm-opt (follow-on to PR#938)

- (id 3687007500, select.rs:19720) "This comment is meant to be time-stable, but it still contains
  time-relative wording ('not yet implemented', 'FUTURE leak-fix'). Consider switching to strictly
  present-tense wording ('is not implemented' / 'a leak-fix') to avoid it going stale again."

### Liaison verification (confirmed on trunk eb092a36e)

The PR#938 reword (`3ec5ceeb4`) DID land — the sha shorthand is gone and it now says "the general Perceus
param-drop pass is not yet implemented in this backend; a known gap tracked by v-memory-safety". But
Copilot flags RESIDUAL time-relative phrasing: "not YET implemented" (the "yet" still implies imminent
change) and "a FUTURE leak-fix that legitimately reclaims…". Copilot's finer polish: strictly present
tense — "is NOT implemented in this backend" (drop "yet") and "a leak-fix" (drop "FUTURE"). Genuinely more
time-stable. Low-priority follow-on to the PR#938 fix. Comment-only, behavior-neutral.

Owner 2: **v-wasm-opt** (select.rs golden, their `2a79a7df4`/`3ec5ceeb4`). Optional finer reword to strict
present tense.
