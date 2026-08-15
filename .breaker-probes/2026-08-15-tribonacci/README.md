# trb1 — tribonacci with window readout (2026-08-15, tick 1504)

3-tuple (a,b,c) rolling recurrence: step → (b,c,a+b+c) answering the new
term; peek answers the live window sum WITHOUT advancing (a pure observer
between steps — pins that peek leaves the orbit untouched: rows 3 and 6
equal the sums the next step then produces). Seed shapes the leading term
(n%3: 1 vs 0), shifting the whole orbit.

Extends fib1's 2-term twin (batch ~241) to 3 terms + observer op. F24-safe:
6 dispatches, ZERO branches, and the 3-tuple is fine because branch-free
(the danger product needs the branching arm).

PASS ×3 wasm. **Pool — fills the dbt1/sfu1/trb1 trio.**
