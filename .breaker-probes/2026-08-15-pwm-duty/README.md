# pwm1 — PWM duty cycle: F24 body-size face + a SHARP contrast pair (2026-08-15, tick 1502)

(phase, duty, on-count) 3-tuple: tick answers on/off by (% phase 4) < duty
(2-branch), advancing phase and counting; setduty retunes mid-stream
(branch-free). 9 dispatches (7 ticks + 2 setduty).

INVALID WASM ×3: 56,906,488-byte emit, wasm-tools 'function body size count
exceeds limit' (dst's BODY-SIZE kind).

## The contrast pair that sharpens the coverage matrix
lap1 (batch 279, LANDED GREEN): 3-tuple × 9 dispatches × a THREE-branch
nested-if arm (lap) + branch-free tick/bst — PASSES.
pwm1 (this): 3-tuple × 9 dispatches × TWO-branch tick + branch-free setduty
— EXPLODES at 57MB.
So branch COUNT alone doesn't predict it; nor tuple width × dispatches alone
(lap1 has both). Candidate discriminator: WHICH arm carries the branches ×
how many dispatches hit THAT arm (pwm1: 7 of 9 hit the branching tick; lap1:
only 3 of 9 hit the branching lap). Per-dispatch duplication of the BRANCHING
arm's body would explain both.

Held from corpus; on the F24 watch. Fourth natural F24 hit.
