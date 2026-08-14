# 2026-08-14 Josephus elimination (tick 1452)

- `jos1.sexp` — the ring is a (list, cursor) pair: each elim advances the cursor
  k-1 modulo the SHRINKING length (the divisor changes per dispatch as
  eliminations remove elements), reads the victim, drops it by index-filtered
  rebuild. The k comes from the SEED (captured by the arm from the enclosing
  main's n — arm-reads-enclosing-param face). n=2 eliminates 2,4,1; n=3
  eliminates 3,1,5. Modulo-over-shrinking-divisor + cursor-position carry
  between dispatches. PASS ×3 (20401/30105).
