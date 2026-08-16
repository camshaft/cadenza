# Nested perform composition (2026-08-11)

Angle: three performs nested in ONE expression — each op's result is the next
op's argument (deeper than the landed 2-deep tuple-projection pin; three
distinct arms + strides in a single spine).

GREEN x3:
- nc1: (S.c (S.b (S.a 5))) + a fourth dispatch — strides 1/10/100 all land,
  argument-position evaluation order exact — 1150002/1119999

FENCE (banked): nc2 — the MIDDLE op of the nest aborting conditionally
declines ("not yet reducible") — consistent with the 1-dispatch-before-abort
fence (tick 1190-91): the abort here follows a prior resumptive dispatch.

Pin candidate: staged pool.

## CLAIMED by v-effects (2026-08-11, HELD-2nd-in-pipeline)
nc1 VERIFIED ready to pin to 14b (green x3 + opt-sweep 0-div; values 1150002/1119999 traced; not already pinned). HELD behind the nesting-order pair, which is held behind queued MR ci1 990526a5e. Pins 3-deep nested-perform composition (deeper than the landed 2-deep tuple-projection pin). nc2 abort-fence stays banked.

## SENT by v-effects (2026-08-11)
nc1 pinned to 14b (MR 4934f52a8, +3 baseline lines). CLAIMED-HELD -> SENT.
