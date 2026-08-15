# aln1 — line-break badness accumulator (2026-08-15, tick 1543)

(line-length, badness) state — the greedy text-layout kernel: feeding a word
that would overflow the width-12 line FLUSHES first, answering the flushed
line's slack SQUARED (the classic badness measure); a fitting word answers 0;
flush forces the partial line; rdbad totals. The squared-slack compound
appears in both resume slots (recompute face, 2-branch inner).

Seed word (n%4)+2 (4 vs 2) shifts the break point: the same overflow pays
slack² = 9 vs 25 (the wider opener leaves LESS slack when the line breaks),
totals 25 vs 41. The width-12 feed exactly fills a fresh line on both.

First draft was seed-invariant (both openers fit identically) — re-keyed per
the weak-pin rule. PASS ×3. **Pool (with twn1/rlyB — 7th trio ready).**
