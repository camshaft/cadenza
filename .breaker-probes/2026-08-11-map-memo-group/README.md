# Map-keyed memo group (2026-08-11) — edges of the b82e4de01 pin

Angle: the landed 3-SCC memo pin abstracts the cache as a SCALAR (get id
ignores id). The REAL memo shape — a Map-keyed CHAMP cache where get looks up
BY id and put inserts BY id — is the compiler-ml type_of shape for real.

GREEN x3:
- mg3: Map-keyed memo, single top-level demand — hit short-circuits the
  recompute, three defs thread the CHAMP cache — 5/5

FENCE (consistent with tick-1221 mutual-SCC x multi-call):
- TWO top-level type-of calls decline (scalar or Map state alike) — the
  multi-call fence applies to memo groups too. mc3's fence generalizes: it's
  CALL-COUNT on the group, not state shape.

## mg4 + observability limit (tick 1246)
- mg4 GREEN x3: the doubling recompute demands each child twice inside ONE
  demand tree (hit path fires mid-tree) — 40/5.
  HONEST CAVEAT: the SUM cannot distinguish memoized from recomputed (2^k*5
  either way) — mg4 pins the group-fold shape/termination, not memo-ness.
- mg5 (write-count via a SECOND effect to make hits observable): DECLINES —
  a foreign effect inside the memo group exceeds the group fold. Banked as
  the observability fence; a memo-ness witness needs the fold to admit a
  cross-effect group member first.
