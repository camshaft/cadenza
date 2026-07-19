# pr551 — cdz-smith differential oracle: sign-inversion bug + run-rust exit-contract ripple (4 comments)

Mirrored from GitHub PR #551 review comments.
PR: https://github.com/camshaft/cadenza/pull/551 (5-MR publish batch — new cdz-smith differential oracle)
Files: `cdz-smith/src/differential.rs` (3) + `cdz/src/main.rs` (1).

One coherent story: `cdz run-rust` changed its exit contract to return NON-ZERO for usage errors
(missing/ambiguous `--call`, bad `--call`, arg-taking exports). That ripples into two stale doc
comments and an oracle-soundness fix. Plus one independent correctness bug (float sign inversion)
in the same new differential.rs.

## Comment 1 — id 3607145467 [amazon-q] differential.rs:47 — INVERTED sign → double negation
> :stop_sign: **Logic Error**: The condition logic is inverted. `is_sign_negative()` returns
> `true` for negative numbers, so this condition emits a negative sign when the number is already
> negative, resulting in double negation (e.g., `-(-1.0)` = `1.0`). This causes incorrect code
> generation for negative floating-point literals. The condition should be `!val.is_sign_negative()`
> to emit the negative sign only for positive values before negation.
> ```suggestion
>                 if !val.is_sign_negative() {
> ```
CORRECTNESS CLAIM — amazon-q source (has a hallucination history), VERIFY against fresh build:
confirm the branch really emits a `-` prefix and whether the value is later negated (double-neg).
If real, negative float literals render wrong in the differential renderer → bogus mismatches.

## Comment 2 — id 3607148745 [Copilot] differential.rs:210 — stale run-rust exit-contract doc
> The doc here says `cdz run-rust` exits 0 for any verdict and non-zero means a harness
> read-failure, but `cdz run-rust` now uses non-zero exit codes for usage errors (e.g.
> missing/ambiguous `--call`, bad `--call`, arg-taking exports). Update this comment so the
> differential oracle's contract matches actual `run-rust` behavior.

## Comment 3 — id 3607148758 [Copilot] differential.rs:237 — oracle SOUNDNESS: non-zero → Declined
> `cdz run-rust` can exit non-zero for usage errors (e.g. multiple exports without `--call`), but
> this path currently returns `Err(...)` which bubbles up as `Diff::Unavailable` and disables the
> differential oracle for such programs. Treat non-zero exit as `Side::Declined` so the oracle
> stays sound (never mismatches on non-comparable runs) without being marked unavailable.

## Comment 4 — id 3607148779 [Copilot] cdz/main.rs:709 — same stale exit-contract doc (source side)
> This comment claims the sole non-zero exit path is a source read failure, but `run-rust` also
> returns `ExitCode::FAILURE` for usage errors (e.g. multiple exports without `--call`, bad
> `--call`, arg-taking export). Please update the comment so it doesn't misstate the tool's
> exit-code contract.

## Triage + owner suggestions
- #1 (sign inversion): correctness bug in cdz-smith differential renderer — fuzzer/cdz-smith
  territory. VERIFY first (amazon-q).
- #2/#4 (stale exit-contract docs): v-cdz-tooling owns run-rust + its contract; both docs describe
  it. NOTE these landed BECAUSE of the run-rust usage-error exit codes v-cdz-tooling just added
  (follow-on to PR#547 run-rust contract work).
- #3 (oracle soundness): fix is in cdz-smith (treat non-zero as Declined) but driven by the
  run-rust contract — needs coordination between the oracle owner and v-cdz-tooling.
Filing to PM to split owners rather than guess.

---
ROUTED (corpus-bugfix 2026-07-18): findings 1(VERIFY-amazon-q-may-hallucinate)+3(soundness Side::Declined) -> fuzzer (owns differential oracle); findings 2+4 (run-rust exit-contract doc-drift) -> v-cdz-tooling (fold into next commit). Line numbers shifted post-PR#551-merge; verify loci.

---
RESOLVED-PENDING-MERGE (fuzzer, 2026-07-18, MR 4c1959f50):
- (1) amazon-q is_sign_negative sign-inversion = CONFIRMED HALLUCINATION — differential.rs has NO float
  handling / no is_sign_negative / nothing at line 47 (mid doc-comment). No change. (The VERIFY caveat
  I flagged was warranted — matches amazon-q's hallucination history.)
- (3) SOUNDNESS FIXED — a non-zero `cdz run-rust` exit (also a per-program usage error per PR#547) now
  maps to Side::Declined (non-comparable → oracle skips, stays sound) instead of Diff::Unavailable;
  Unavailable/Err reserved for true infra failure. + unit test (non-zero exit → Ok(Declined)).
- (2) stale run-rust-exit doc comment in differential.rs updated too.
Gate green (30 tests, clippy/fmt clean, libFuzzer links). NOTE: finding (4) cdz/main.rs:709 doc-drift is
SEPARATELY with v-cdz-tooling. Retire this file once 4c1959f50 lands.

---
LANDED + VERIFIED (corpus-bugfix 2026-07-18): differential.rs on trunk a26f90b59 has Side::Declined(_) on
either side -> Diff::Agree (skip) (line 134) + the doc table pins Declined=skip — the soundness fix (3)
(non-zero run-rust exit -> Side::Declined not Diff::Unavailable) is present. (1) was a confirmed amazon-q
hallucination (no float code). Fully resolved.
