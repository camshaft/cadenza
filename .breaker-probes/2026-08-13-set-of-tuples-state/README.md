# 2026-08-13 set-of-tuples state (tick 1383)

- `stt1.sexp` — handler state (Set (Tuple Int64 Int64)) grown per dispatch:
  the repeated pair (n,1)×2 does NOT grow the set (structural dedup through the
  thread), the order-swap (1,n) only counts when n≠1 (n=1 seed makes it a THIRD
  duplicate — the seed flips which inserts are no-ops). Existing tuple-set
  coverage rides sets as op ARGS (14:3282, order-identity) — no Set-of-tuples
  handler STATE. NOTE (corrected tick 1384): `Set.empty` does not EXIST in the prelude at all
  (only Map.empty does — known language gotcha): the CDZ0201 on my first draft
  was the unbound name, NOT a compound-element decline surface. Scalar
  `(: Set.empty (Set Int64))` declines identically. Set.of is the only seed.
  PASS ×3 (2234/2223).

## Tick 1384 addition
- `sos1.sexp` — SET-OF-STRINGS state: the arm builds keys by String.concat (ROPE)
  and inserts against a FLAT seed literal "k-e" — rope-vs-flat structural dedup
  through the state thread, parity routes which inserts no-op (n=2: first tag
  dedups vs seed → 122; n=3: it's fresh → 222). CONTENT-equality of rope and
  flat strings as CHAMP set keys across dispatches. PASS ×3.
Also mapped this tick: `(: Map.empty (Map Int64 (Tuple Int64 Int64)))` seed WORKS
(compound VALUES fine) — confirming the Set.empty issue is just the missing
prelude name, no compound-decline surface anywhere here.
