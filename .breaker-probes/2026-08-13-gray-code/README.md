# 2026-08-13 Gray-code generator (tick 1441)

- `gry1.sexp` — the arm answers the counter's Gray encoding `(^ c (>> c 1))`
  then advances; the BODY xor-popcounts consecutive answers, proving the
  single-bit-change LAW holds through the effect boundary (both pc digits
  always 1, regardless of seed — the law IS the seed-invariant part; the g
  values themselves differentiate 267/552). XOR-of-shift in the arm + the
  body-side property check via recursive popcount. PASS ×3 (267011/552011).
