# mrt1 — metronome with a downbeat accent (2026-08-15, tick 1579)

(beat, bars) state: `tick` advances wrapping at the seed-shaped bar length
((n%3)+3: 4 vs 3), counting bars; `accent` answers 100+bars ON a downbeat or
the beats remaining until the next one (the countdown compound
(- (+ (% n 3) 4) beat) — seed arithmetic in the non-accent branch).

Bar lengths 4 vs 3 shift the phase: the same tick positions land the second
accent ON the downbeat for the 4-bar (101 — bar count in the answer) and off
it for the 3-bar (2 — countdown), while the first accent inverts (2 vs 1
countdowns). Phase alignment as the differential — the same probe points
sample different bar positions.

PASS ×3. **Pool (with lgt1; +1 fills the 11th trio).**
