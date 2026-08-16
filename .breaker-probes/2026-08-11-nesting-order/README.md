# Nesting-order sensitivity of cross-arm performs (2026-08-11)

Angle: an inner handler's arm performing the OUTER effect is pinned in the
resume-value position (the 51-forward); the mid-TRANSITION face (the perform
inside the arm's answer computation, advancing BOTH threads per inner
dispatch), and its order-flipped REJECT twin, make a teaching pair.

GREEN x3 (the pair):
- no1: B's arm computes (+ t (A.ga)) — each inner dispatch advances both
  threads in lockstep — 114103/111100
- no2: the SAME program with nesting FLIPPED (B outside) is CDZ0401 no-home
  — arm bodies resolve under the handlers enclosing THEIR handle, so B's arm
  can't see the inner A. The reject twin pins the scoping rule the green
  case relies on.

Pin candidates: staged pool (as a pair).

## CLAIMED by v-effects (2026-08-11, HELD)
no1+no2 pair VERIFIED ready to pin to 14b (no1 green x3 + opt-sweep 0-divergence + values 114103/111100 traced; no2 CDZ0401 reject stable; neither already pinned). HELD from sending — v-effects has a queued MR (ci1 990526a5e) not yet landed; no-layer-on-queued-MR discipline. Will pin as a pair the moment ci1 lands.

## PINNED by v-effects (2026-08-11)
no1+no2 pair PINNED to 14b (MR bfc7fcf51, +2 baseline lines x3 backends). CLAIMED-HELD -> DONE.
