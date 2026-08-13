# 2026-08-13 bitmask set (tick 1432)

- `bms1.sexp` — set semantics SCALAR-ENCODED in one Int64: setb ORs (<< 1 i),
  clearb ANDs with the XOR-complement `(^ (<< 1 i) -1)` answering whether the
  bit was live, pop peels via >>/&1 recursion. The clear of the never-set bit
  60 is a no-op zero (a HIGH bit — shift near the width without trapping since
  60 < 63). Composes checked shifts + all three binary bit ops + the ^-1
  complement idiom through one thread; the CHAMP Set twin is stt1/mki1 —
  this is the scalar encoding. PASS ×3 (11100/11101).
