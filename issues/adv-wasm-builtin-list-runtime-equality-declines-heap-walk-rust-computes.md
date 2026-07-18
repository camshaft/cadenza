# wasm gap: built-in List RUNTIME = declines "heap walk not yet built"; rust computes (equality twin of Symbol/String-ordering)

**Reporter:** breaker (2026-07-18), verified by corpus-bugfix on trunk 9404d08cd. **Severity:** backend divergence (capability gap, not miscompile).

## Finding (genuinely-runtime operands — const would fold the gap away)
```
(def (run (: a Int64) (: b Int64)) (if (= (list a b) (list a b)) 1 0))   ; run --arg 3 --arg 4
  wasm: "comparison of a compound value needs a heap walk (not yet built)"
  rust: value 1
```
CONTROL runtime tuple `(= (tuple a b) (tuple a b))` -> wasm computes 1. So the wasm compound-= heap walk covers Tuple/Map/user-recursive-sums but NOT the built-in List rep.

## Isolation (breaker)
const built-in list folds+computes; runtime tuple/Map/user-IntList-sum all walk on wasm; only runtime BUILT-IN `(list a b)` = declines. Corpus pins runtime list-like = only via a user-defined IntList sum (03-equality:859, different code path) → the built-in-List rep gap is invisible.

## Routing
ROUTED to v-runtime (corpus-bugfix 2026-07-18): the ty_heap_walkable / compound-= heap-walk territory. The EQUALITY twin of the Symbol/String-ORDERING gap they just fixed (17-symbols:103). FIX: add the built-in List rep to the runtime = heap walk (same as Tuple/Map). Once landed, a genuinely-runtime `(= (list a b) (list a b))` -> 1 pin (--arg operands, NOT const) is the guard. Not spawning.

---
SHARPENED (breaker, 2026-07-18): probed all heap types — runtime Set/Bytes/String(rope)/Map/Tuple/user-sum =
ALL COMPUTE on wasm; ONLY built-in LIST = declines. The LONE gap → a SINGLE missing ty_heap_walkable / value-eq
arm for the List rep (all siblings already covered). Small, high-confidence, well-scoped one-arm add for v-runtime.
