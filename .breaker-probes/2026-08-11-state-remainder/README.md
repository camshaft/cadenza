# State-remainder faces (2026-08-11)

Angle: % BY the state (sign faces as the divisor walks negative->positive)
and remainder-driven termination with a bounded fallback.

GREEN x3:
- rm1: truncated dividend-sign remainder holds through negative/positive
  state divisors — 99/98/199 (n=3/-5/2)
- rm2: hunt terminates at the first draw divisible by 7 OR exhausts a
  20-step budget (the guarded-nontermination-safe version of the m3 shape)
  — 700/700/1400

Pin candidates: staged pool.

## CLAIMED by v-effects (2026-08-11, HELD-in-pipeline)
rm1 VERIFIED ready to pin to 14b (green x3 + opt-sweep 0-div; 99/98/199 python-traced incl. negative-divisor faces). DISTINCT from nm1 (14c:619): nm1 divides the STATE by a FIXED 7; rm1 divides the op-value BY the WALKING STATE (divisor crosses negative->positive, sign-of-dividend truncation holds). HELD behind cp2 (behind queued MR dv-pair 09609e60e). rm2 (bounded remainder-search) a further candidate.

## SENT by v-effects (2026-08-11)
rm1 pinned to 14b (MR cd6306075, +3 baseline lines). CLAIMED-HELD -> SENT.

## PRUNED rm1 (tick 1305): v-effects has the remainder-by-walking-state pin
QUEUED at pr-sync (14b). rm2 (bounded remainder-hunt) not covered — stays.
