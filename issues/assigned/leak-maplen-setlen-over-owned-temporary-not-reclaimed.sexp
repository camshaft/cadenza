;; LEAK (no wrong value) (2026-07-15, v-memory-safety): the SIBLING of the List.len/Bytes.len
;; owned-temporary leak (fixed tick 18). Map.len (MapSize), Set.len (SetLen) — and the COLLECTION operand
;; of Set.contains / Map.lookup — over an INLINE OWNED-TEMPORARY collection do NOT reclaim it: the
;; borrowing op reads a scalar/bool and returns, nothing drops the temporary → 1 heap cell/call leaked.
;; Value is CORRECT (leak-only). Map.len/Set.len emits (select.rs ~5963/6038) do `emit(map); map-size`
;; with NO reclaim (unlike ListLen after tick 18); Set.contains/Map.lookup (~6117/6165) drop the owned
;; KEY/ELEM temporary via key_handle_is_owned_temporary but NOT the owned collection operand.
;;
;;   (Map.len (build 0 n (map)))   (Set.len (build 0 n (Set.of (list))))   -- built, borrowed, LEAKED
;;
;; FIX: same as the ListLen fix (tick 18) — in MapSize/SetLen emit, if heap_operand_ownership==Owned,
;; stash the collection in a scratch slot across the borrowing size op, then drop it; register OP_DROP in
;; the collect arm. For Set.contains/Map.lookup, add the analogous collection-operand reclaim alongside the
;; existing key/elem drop (rarer shape: `(Set.contains (build-set) x)`). TERRITORY: v-memory-safety.
(do
  (def (build (: i Int64) (: n Int64) (: m (Map Int64 Int64))) (if (< i n) (build (+ i 1) n (Map.insert m i i)) m))
  (def (main (: n Int64)) (Map.len (build 0 n (map))))
  (export main))
