# ftn1 — wishing fountain with city skimming (2026-08-17, tick 1685)

Attack: a RESET-TO-CONSTANT clamp with the excess ACCUMULATED elsewhere — the
skim branch answers `(- (+ coins v) 8)` (the excess), stores constant 8 into
coins, and adds the same excess compound into skimmed (the compound in answer
+ ONE rebuild field but not the other — the tol1 inverse-shape at a clamp).
Scoop is a floor-min pair (under-3 zeroes; else exact-3) with the leftover's
low digit in the take branch's answer.

Differential: pool 9 vs 5: n=10's FIRST toss skims (12→... 9+3=12 not >12 —
wait: 121 = plain 12 at wish 1; skim on toss #2's +4: 13... rows show 752 at
position 3 = skim of 5 on the SECOND toss (9+3=12 plain, scoop→9, 9+4=13 →
skim 5); n=0 crosses on the LAST toss (11+2=13 → skim 5? rows show 113 plain
at position 4 — n=0 never skims! read 1103: skimmed 0... 11*100+0+3 = 1103 ✓).
One run skims once, the other never — the 700-branch is seed-exclusive.

Hand model: n=10 → 1210397521031053; n=0 → 810350921131103 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 13f6dd0b1.
