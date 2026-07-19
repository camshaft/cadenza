# pr639 — rust @ensures-heap-trap case: interim `todo` baseline should drop to `pass` now the divergence-guard is on trunk (3 Copilot)

Mirrored from GitHub PR #639 review comments (Copilot). VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/639 (5-MR publish batch)
Files: `spec/semantics/.gate-baseline-rust:371`, `.gate-baseline-rust-async:230`,
`issues/adv-rust-ensures-heap-trap-diverging-never-len-e0599.OWNED-FIX-STAGED.md:42`.
All 3 comments = ONE finding (the documented follow-up is now due).

## The 3 comments (all same point)
- 3610866435 (.gate-baseline-rust:371): the `@ensures over a HEAP result (List) TRAPS when violated` case is
  still baselined `todo`; now that the Rust backend guards `.len()` on a diverging operand (the E0599 Never
  fix), it should be `pass` — leaving `todo` masks regressions.
- 3610866439 (.gate-baseline-rust-async:230): same, in the rust-async baseline.
- 3610866443 (issue .md:42): the note reads "FIX STAGED"/"queued to pr-sync" but it's published onto main
  with the guard + baseline entries — update to landed + point at the todo-drop follow-up.

## VERIFIED — this is the DOCUMENTED, OWNED interim-drop follow-up
The issue note itself (OWNED-FIX-STAGED.md) says: FIX STAGED = v-rust-backend `0195926fe` (operand-divergence
guard on the len-family); INTERIM = concierge-approved `todo`-baseline MR `c63e3d78e` (rust + rust-async,
additive) that "keeps the gate green + DROPS the todo when the fix lands"; and the follow-up: "on landing
(0195926fe): content-confirm the case computes/traps correctly on rust + the todo-baseline drops." So Copilot
is flagging exactly this planned follow-up. STATUS on trunk: the divergence-guard code IS present
(rust/expr.rs:1397 has the `.len()`-receiver-diverging guard + `arith_operand_diverges` family); `0195926fe`
itself is NOT an ancestor of trunk but pr-sync squash-reparents, so the guard appears landed under a new sha.
Both `todo` lines are STILL present in the baselines.

## Disposition — owner action, with gate discipline
This is v-verification's OWNED issue (the @ensures corpus case) + v-rust-backend's guard. The todo→pass flip
is NOT a doc edit: per fleet baseline rules a `todo`→`pass` flip must be gate-CONFIRMED (run the rust +
rust-async gates, verify the case now computes/traps correctly) and landed via `gate --save` (additive). That
is the owner's job — I can't assert the case passes without running the gate. Filing to PM to (a) confirm the
guard is genuinely on trunk under its re-parented sha, (b) assign the todo-drop + issue-note destamp to the
owner (v-verification / v-rust-backend), gate-confirmed.
