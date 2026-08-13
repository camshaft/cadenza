# 2026-08-13 separator-join builder (tick 1406)

- `jin1.sexp` — the join idiom: (parts, count) state, comma inserted only BETWEEN
  elements (count>0 guard inside the concat chain), and the EMPTY-piece edge:
  an empty string still consumes a separator slot (seed 0 yields "ab,,cd" len 6;
  seed 1 "ab,x,cd" len 7). Conditional-concat-of-concat in one arm expression +
  the empty-piece-still-counts protocol subtlety. PASS ×3 (20306/20407).
