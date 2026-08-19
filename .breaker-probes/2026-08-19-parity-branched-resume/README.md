# parity-branched-resume — arm branches the whole resume (answer + next-state) on state parity
## pypb1 — even: (* s 10)/+3, odd: (+ s 100)/*2; 3 dispatches walk alternating parity. Model 105020101/60103000. PASS x3.
Confirms an if over (% s 2) selecting BOTH the resume answer and next-state compiles correctly;
the next-state choice steers the following dispatch's parity (both branches fire). Promotable.
