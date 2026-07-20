# pr662 — music: tick-instant UInt64.wraps a negative tick + "12 chromatic notes" test has only 6 (2 Copilot)

Mirrored from GitHub PR #662 review comments (Copilot). Both VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/662 (v-music Phase-2 demos) — v-music territory.

## #1 — id 3612250840 (des-piece.cdz:53) — tick-instant wraps a negative Int64 tick to a huge UInt64
> `tick-instant` uses `UInt64.wrap(tick)`, which wraps negative Int64 values (e.g. -1 → 2^64-1). Since
> `MidiEvent` ticks are Int64 (and `schedule`/`note-at` don't enforce non-negative starts), a negative
> start/duration could make the simulated clock jump to a huge instant instead of failing fast.

VERIFIED: `def tick-instant(tick: Int64) = Instant.Instant(UInt64.wrap(tick))` (des-piece.cdz:53). The doc
says "Int64, always >= 0 in a played piece" — but that's an ASSUMED invariant; `note-at`/`schedule` don't
enforce non-negative starts/durations, so a negative tick `UInt64.wrap`s to ~2^64 and the DES virtual clock
jumps instead of failing fast. Robustness gap (defensive check missing). Fix options (v-music's call): guard
tick >= 0 (trap/decline on negative), or enforce non-negative at note-at/schedule. Severity depends on
whether a negative tick is reachable in the demo path — owner judgment.

## #2 — id 3612250859 (pipeline.cdz:96) — "all 12 chromatic notes" test actually has 6
> This test claims to feed "all 12 chromatic notes from C4", but `chromatic-run` currently includes only 6
> notes (60,61,62,63,64,66). That weakens the intended coverage and makes the comment/test name misleading.

VERIFIED: test `a-whole-chromatic-run-collapses-into-major` (pipeline.cdz:89), comment "Feed all 12
chromatic notes from C4", but `chromatic-run` = notes 60,61,62,63,64,66 — SIX notes, and it even SKIPS 65
(and 67-71). So neither 12 nor contiguous-chromatic. Either fill it to the real 12 chromatic (60..71) to
match the "whole chromatic run" intent, or rename/reword to what it actually tests. Weaker coverage than
the name claims. → v-music.

## Owner
Both `implementation/music/*` = v-music. #1 robustness (negative-tick wrap), #2 test coverage/naming.
