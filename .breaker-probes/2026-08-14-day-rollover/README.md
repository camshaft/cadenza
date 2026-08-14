# 2026-08-14 day-rollover ledger (tick 1454)

- `day1.sexp` — (balance, buffer, day) triple: txns accumulate in the buffer
  (incl. a NEGATIVE -30), endday POSTS the net into the balance, bumps the day,
  answers day*1000+net+100, and CLEARS the buffer so day 2 starts clean (its
  endday nets exactly 5 regardless of seed — the seed-invariant rows PROVE the
  clear, while day-1 rows carry the seed). Near-i64-width packed expectations
  (16-17 digits, verified < 2^63). Period-boundary accounting: accumulate →
  post → reset, the flush-with-summary face (dbn1's flush releases ONE stash;
  this nets a whole period). PASS ×3.
