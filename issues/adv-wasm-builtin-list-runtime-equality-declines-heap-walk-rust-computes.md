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

---
REFINED (breaker, 2026-07-18) — LOWER-RISK FIX: the List equality capability ALREADY EXISTS (used for Map
KEYS + Set ELEMENTS — proven: (list 5 5) dedups as a Map key to len 1; {[3,8],[8,3]} as Set elems to len 2).
It is just NOT wired to the standalone = operators compound-value dispatch (ty_heap_walkable / value-eq),
even though the CHAMP key-comparison path covers List. FIX: route = on a List through the SAME list-compare
the Map/Set key path already uses (dispatch-wiring add), NOT a fresh walk — smaller + lower-risk. List is
the lone operator-dispatch omission; all other collections route = to their walk fine.

---
BLAST RADIUS (breaker, 2026-07-18) — PRIORITY-RAISER: the missing List arm blocks = on ANY compound
CONTAINING a list (the walk recurses into the list element + hits the missing arm), not just top-level
list =. Verified wasm: (= (Option.Some (list a b)) …) DECLINES; (= (tuple (list a b) 9) …) DECLINES; rust
computes both; control (tuple, no list) computes. So = on a record-with-a-list-field / Option-of-list /
tuple-with-a-list is unusable on wasm. The SAME one-arm dispatch fix clears the whole family at once.

---
⚠ SOUNDNESS CORRECTION (v-runtime, 2026-07-18) — the champ_eq one-arm flip is a MISCOMPILE, DO NOT apply:
champ_eq is a PHYSICAL byte/structural walk, correct ONLY for a byte-canonical rep. A CHAMP map/set IS
byte-canonical; an RRB VECTOR (List) is ELEMENT-canonical NOT shape-canonical (concat-built vs push-built
lists with identical elements have DIFFERENT internal shapes — relaxed interior nodes + packed-bool leaf;
runtime lib.rs:20834-42 + 3650 document this, its own fuzz refuses to assert champ_eq for vecs). Routing = through
champ_eq -> [1,2,3](concat) == [1,2,3](push) returns FALSE = silent equality miscompile. That's WHY
ty_heap_walkable returns false for Ty::List BY DESIGN + why a List is never a map/set key. The "capability
exists via Set/Map dedup" was for map/set STORAGE (canonical CHAMP), NOT a List value/element compare.
SOUND FIX: an ELEMENT-WISE walk (compare vec-len, then op_vec_get each index in order — shape-independent),
a sibling of the value_cmp_shaped v-runtime built for slice-2 (compound ordering). So sound List = is a real
INCREMENT (~value-cmp shape, not a one-liner), QUEUED AFTER slice-2. Blast radius (Option-of-list,
tuple-with-list-field) all need the same element-wise list leaf; the increment clears them. NOT the earlier
one-arm flip — that passes same-shape tests but miscompiles concat-vs-push twins.

---
DATA POINT (breaker, 2026-07-18, v-runtime's call on relevance): the Map/Set LIST-KEY path already compares
lists SHAPE-INDEPENDENTLY + correctly TODAY (concat-built key = push-built lookup -> same key, even n=40;
[1,2,3] vs [1,2] stay distinct). So NOT naive champ_eq. Either (a) RRB is canonicalized-on-construction
(shape concern moot for current rep) or (b) the key path routes through a shape-independent element-wise
compare separate from champ_eq — which might be the exact compare to wire to standalone = (reuse, NOT via
champ_eq). Could shortcut the from-scratch element-wise walk. v-runtime's call. Does NOT revive the champ_eq
route (still vetoed) — this is about whatever the KEY path actually uses.

---
SETTLED (v-runtime empirical probe, 2026-07-18): the "key path already compares shape-independently" data
point REVERSED — concat-vs-push twins: champ_eq TRUE for n<=32 (leaf merge = strict leaf, shape-canonical)
but FALSE for n>=33 (concat leaves a RELAXED interior node w/ size table, push builds a strict trie →
different bytes → champ_eq false at every split boundary). breaker's "same key at n=40" only held because BOTH
lists were built the same way. So NO shape-independent list compare exists in the key path for n>=33; champ_eq
STAYS VETOED. List = requires the ELEMENT-WISE walk (value_eq_shaped via op_vec_get, reusing slice-2 value-cmp),
queued after slice-2. No shortcut.

---
LANDED (List<orderable>) + verified (corpus-bugfix 2026-07-19): built-in List `=` for ORDERABLE-leaf lists
shipped on trunk `ae2eb02eb` (option A — route `Prim::Eq && is_orderable_compound` through `Core::ValueCmp{op:Eq}`,
element-wise value-cmp walk, res==0; NO hash bump). Verified fresh-build, runtime operands: concat-built
vs push-built `[k,k+1,k+2]` with `--arg 7` → `= ` returns 1 (equal, shape-independent) on wasm. Covers
List<Int/String/BigInt/Rational/Bool/tuple/sum-of-those>. champ_eq route stays VETOED.
RESIDUAL (still open, HELD): `List<Float>`/list-spine-containing-Float `=` still DECLINES (value-cmp excludes
non-orderable leaves per §319). BRICK 1 core `value_eq_shaped` landed hash-neutral (`c41d3e368`); BRICK 2
(export as a runtime op + emit routing) is HELD by concierge (C)-batch for the NEXT hash-bump window — reject-
don't-miscompile meanwhile, no forcing consumer. WATCH: ride a hash-bump landing or a forcing consumer; do
NOT solo-bump. This file stays OPEN for the List<Float> residual only.
