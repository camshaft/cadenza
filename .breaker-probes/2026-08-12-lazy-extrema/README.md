# 2026-08-12 lazy-init extrema tracker (tick 1350, base post-243 trunk)

- `mmx1.sexp` — state `(Option (Tuple Int64 Int64))` extrema tracker: starts None,
  first feed initializes (v,v), later feeds widen via nested Option→tuple matches,
  range answers max-min (uninitialized → 0). Combines the olc1 lazy-lifecycle idiom
  with a SCALAR-PAIR payload (olc1's payload is a heap List; this is the flat-tuple
  face — different boxing path). Feeds ordered so mins and maxes both move:
  n=5: ranges 0,0,2,7,7 → 20707; n=0: 0,0,3,10,10 → 31010. PASS ×3.
