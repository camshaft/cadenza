# Draw-determined depths + FINDING #19 (2026-08-11)

GREEN x3 (pin candidate):
- sd2: the recursion DEPTH is a draw (mod-4 of state), incl. zero-depth face
  — 200078/0/100006

FINDING #19 (filed): nested recursive performers drop the INNER's out-state.
- outer draws a depth per iteration, calls inner(depth) which ticks per hop;
  the inner's advances are LOST at the outer recursion boundary.
- Exact drop-model verified at K=1/2/3 outer iterations x seeds 0/1 — the
  compiler matches the dropped model at EVERY point (K=3 n=1: 9 vs correct 7).
- Uniform x3 backends (differential vs python reference, not cross-backend).
- Queue: adv-inner-walk-out-state-dropped-at-outer-recursion.sexp (correct-
  threading pins). Kin of #13 peel_resume / tk3d under-report.
- sd3 (the finder probe) held until fix; sd2 promotable now.

## #19 scope controls (tick 1261)
- s19a: FIXED inner depth under outer recursion — DROPS identically (6 vs
  correct 10) — the drawn depth is irrelevant.
- s19b: TWO sequential inner calls from the BODY — threads correctly
  (703/501). Trigger = inner recursive performer called from an OUTER
  RECURSIVE fn; body-position callers demand the out-state fine.
Fix surface: the recursive-caller path of the out-state demand analysis.

## #19 controls round 2 (tick 1263)
- s19d: operand order swapped ((+ (inner 2 0) acc)) — drops identically.
- s19e: indirection def (via k -> inner k 0) — drops identically; the demand
  miss survives a call-graph hop.
- s19c: let-bound inner BEFORE the recursion (non-tail) — honestly DECLINES
  (no miscompile on that shape).

## #19 fix verification (tick 1269) — PARTIAL
Fix 5c419c8bf: direct-call shapes (sd3/s19a/s19d) now DECLINE honestly (sound
floor — no more silent drop on those). BUT s19e (one-hop INDIRECTION:
outer -> via -> inner) STILL silently miscompiles (6 vs 10) x3 — the
recursion-boundary-observed marking stops at the direct callee. Reported;
follow-on needed (walk the callee chain or decline conservatively).

## #19 follow-on scope (tick 1270)
- s19f: TWO-hop indirection (via1->via2->inner) drops identically — the miss
  is depth-general.
- s19g: STRAIGHT-LINE performing helper (no recursion) under the same outer
  recursion threads correctly (10/6). Gap: indirection chains whose LEAF is a
  RECURSIVE performer, any depth. The chain walk must reach recursive leaves.

## CLAIMED by v-effects (2026-08-11, HELD-3rd-in-pipeline)
sd2 VERIFIED ready to pin to 14b (green x3 + opt-sweep 0-div; values 200078/0/100006 traced incl. zero-depth face; not already pinned). HELD behind nc1 (held behind queued MR bfc7fcf51). sd3 finder-probe + FINDING #19 already soundness-closed+pinned separately.

## SENT by v-effects (2026-08-11)
sd2 pinned to 14b (MR e480379f8, +3 baseline lines). CLAIMED-HELD -> SENT.
