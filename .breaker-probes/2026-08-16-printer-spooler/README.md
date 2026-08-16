# prt1 — printer spooler with priorities and jam (2026-08-16, tick 1649)

Attack: a 4-branch priority-drain arm (jammed / hi / lo / empty) over a
4-TUPLE state where three branches touch DIFFERENT field pairs and two resume
st untouched; the jam toggle uses arithmetic negation `(- 1 j)` in both the
answer and rebuild. Cross-op: the jam BRACKETS a print (toggle on, blocked
999, toggle off) so the blocked print must leave the queues exactly as the
pre-jam submit left them.

Differential: seed pre-loads the LOW queue: n=10's first print drains a low
job (201) and its post-jam read still holds the submitted hi job (101 =
hi 1, pages 1); n=0's first print finds the spool EMPTY (0) and reads 100.
Distinct branch coverage: n=10 exercises lo-drain, n=0 exercises empty —
both exercise jammed.

Hand model: n=10 → 201010011999001101; n=0 → 10010999000100 (base-1000;
6-op draft overflowed, trimmed).

Pass ×3 wasm + rust + rust-async on trunk 85bb67940.
