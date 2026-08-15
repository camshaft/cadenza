# rmn1 — Roman-numeral accumulator with subtractive correction (2026-08-15, tick 1524)

(total, prev) state: feeding a numeral LARGER than its predecessor
retro-subtracts the predecessor twice (the IX/IV rule: total += v - 2*prev);
otherwise plain addition. tot reads. Seed picks the second numeral (5 vs 1
via an if in the CALL ARGUMENT — the seed expression lives in the perform,
not the arm): X,V,X,I,V vs X,I,X,I,V. The correction fires at position 3
only for n=0 (1<10 retro-subtracts) and at position 5 for both (1<5), so
the totals CROSS (19 vs 23 — the n=0 run ends HIGHER despite feeding the
smaller numeral).

3-branch arm, 6 dispatches, 2-tuple, cheap recomputes — envelope-safe.
PASS ×3. **Pool — fills lck1/bwl1/rmn1 (sixth trio ready).**
