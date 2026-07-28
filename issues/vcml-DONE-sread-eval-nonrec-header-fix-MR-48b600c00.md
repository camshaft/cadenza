# TODO (v-compiler-ml, post-HO-stack-drain): fix sread-eval-nonrec.cdz dangling queue/ ref

**Source:** github-liaison / Copilot review of PR#796 (tick-203). CONFIRMED on trunk. Doc-only, behavior-neutral.

## The nit
`implementation/compiler-ml/src/sread-eval-nonrec.cdz` header (lines 2-3) references
`queue/vcml-probe-2026-07-22-nonrecursive-shapes-all-green-candidate-regression-tests.md` — but the fleet
`queue/` lives under `.claude/` (gitignored, hub-only), so it's a DANGLING pointer for a reader of the tracked
source.

## The fix (2 lines, standalone commit)
Replace the `(see queue/…md)` ref with an INLINE one-line summary of the probe finding, e.g.:
`/// surfaced green by a 2026-07-22 run-ml probe of non-recursive shapes (M bool-helper-in-if-cond→111,`
`/// L param-in-if-cond+negative-arg→0, B multi-binding-let→5 — the highest-value of 14 verified-green shapes).`
Drop the `queue/…md` filename entirely (ephemeral hub state, not a durable link).

## Why not now (tick-203)
My branch carries a 4-deep HO-slice stack (HO-1→2a→2b-i→2b-ii-A) queued/held at the 3-deep MR cap. A doc-fix to
sread-eval-nonrec (UNRELATED to HO) must NOT stack as a 5th commit on unlanded work (mixes concerns + deepens
base-pin). DO THIS once the HO stack lands and the branch resets to trunk → clean standalone 2-line commit + MR.
Acked to github-liaison (they know the sequencing).
