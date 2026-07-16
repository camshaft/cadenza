;; LEAK (no wrong value) (2026-07-16, v-memory-safety): the LAST member of the owned-temporary
;; borrowing-op leak family (List.len/Bytes.len fixed leak-B; Map.len/Set.len fixed tick 20). The
;; COLLECTION operand of Set.contains / Map.lookup, when it is an INLINE OWNED-TEMPORARY, is not reclaimed:
;; these ops drop the owned KEY/ELEM (via key_handle_is_owned_temporary) but NOT the owned set/map operand.
;;
;;   (Set.contains (build 0 n (Set.of (list))) 1)   -- the built set is borrowed by set-contains, then LEAKED
;;   (Map.lookup   (build 0 n (map)) k)              -- the built map likewise
;;
;; VALUE CORRECT (leak-only): set.contains returns 1; only the owned-temporary collection leaks (1 cell).
;; WAT-confirmed: main emits only ONE drop (the key/elem), not two (the collection is not stashed+dropped).
;; FIX: in SetContains/MapLookup emit (select.rs ~6182 / ~MapLookup), stash the collection handle in a
;; second scratch slot right after emitting it (if heap_operand_ownership==Owned), then drop it after the
;; borrowing op alongside the existing key/elem drop; register the extra OP_DROP in the collect arm. A
;; BORROWED param/local collection is left to its owner. TERRITORY: v-memory-safety. Rarer shape (build a
;; collection to query once, then discard) — lower priority than the Len leaks but completes the family.
(do
  (def (build (: i Int64) (: n Int64) (: s (Set Int64))) (if (< i n) (build (+ i 1) n (Set.insert s i)) s))
  (def (main (: n Int64)) (if (Set.contains (build 0 n (Set.of (list))) 1) 1 0))
  (export main))
