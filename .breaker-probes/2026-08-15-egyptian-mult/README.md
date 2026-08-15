# egy1 — Egyptian (peasant) multiplication (2026-08-15, tick 1526)

(multiplier, multiplicand, accumulator) 3-tuple: each step tests the
multiplier's low bit — odd accumulates the doubled multiplicand — answering
odd*100 + the running product's low two digits, then halves/doubles; acc
reads the exact product. Seeds 15×7 vs 5×7: bit patterns 1111 vs 1010(rev
0101) fire the accumulation on ALL steps vs alternating steps
(107,121,149,105 vs 107,7,135,35), products 105 vs 35.

2-branch arm, 4 through it, cheap recomputes — envelope-safe. PASS ×3
(gated after a severe host load storm — 5-min avg peaked 324; waited it
out per the load-kill protocol). **Pool (with dlt1; +1 fills 7th trio).**
