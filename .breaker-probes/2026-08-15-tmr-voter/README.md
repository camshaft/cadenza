# tmr1 — triple-modular-redundancy voter (2026-08-15, tick 1527)

(a, b, c, disagreements) 4-tuple: `set` writes one channel through a 3-branch
index lattice; `vote` answers the majority via the pairwise-equality lattice
(a=b / a=c / b=c) or -1 counting the all-differ case; `bad` reads the count.

Seed corrupts channel 0 (n+2: 12 = agreeing vs 2 = corrupt): n=10 stays
UNANIMOUS through all three votes (never disagrees); n=0 walks majority-flip
(12 by b=c) → full disagreement (-1, count 1) → healed 2-vote majority
(2 by a=c after the seed value lands in channel 2). Vote rows: 12,12,12 vs
12,-1,2 — three distinct voter outcomes on one run.

4-tuple × cheap 3-branch arms; branching arms get ≤3 dispatches each —
envelope-safe (trn1's 7-through-branching was the breaking point). PASS ×3.
**Pool — fills dlt1/egy1/tmr1 (seventh trio ready).**
