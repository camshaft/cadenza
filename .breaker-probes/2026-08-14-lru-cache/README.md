# lru1 — capacity-2 LRU cache (2026-08-14, tick 1485)

Two-op handler over (recency list MRU-last, Map values). `put` re-inserts the
key as MRU via a recursive `without` rebuild, evicting the LRU head when over
capacity (answering the evicted key, else 0) with Map.insert+Map.remove in one
arm; `get` answers the value refreshing recency, or -1 on miss.

Style notes under the known fences:
- All list helpers (without/headof/tailof) are LET-FREE (medK fence).
- The put arm binds the rebuilt recency list with an irrefutable MATCH binder
  (rec2) instead of a let — dual-used for both slots and it compiles; a
  match-binder is not fenced where the callee-let is.

Seed collision: key n+1 == literal 1 exactly at n=0, so the first two puts hit
the SAME slot — n=0 UPDATES (evicts nothing, get 1 → 6, never misses) while
n=10 fills to capacity and EVICTS key 1 on the third put (get 1 → -1):
5009905 vs 6000606 — different eviction, hit/miss, and value rows end-to-end.

PASS ×3 wasm. **Pool (with qrm1).**
