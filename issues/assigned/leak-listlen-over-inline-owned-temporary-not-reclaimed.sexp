;; LEAK (no wrong value) (2026-07-15, flagged by fix-map-set-enum-frontend, root-caused by v-memory-safety):
;; a BORROWING op (List.len / Bytes.len) over an INLINE OWNED-TEMPORARY list/bytes does NOT reclaim the
;; temporary — the sequence is built, borrowed by vec-len (which does NOT consume), and never dropped, so
;; one heap cell leaks per such call. Distinct from a leak-BALANCED borrow of a param/let (the OWNER
;; reclaims those). This is the ListLen/BytesLen twin of the `Core::Proj` reclaim branch (select.rs ~6429),
;; which DOES stash+drop an owned-temporary aggregate after the borrowing arr-get; ListLen/BytesLen (emit
;; ~5282) just does `emit(operand); vec-len` with NO reclaim.
;;
;;   (List.len (build 0 n (list)))   ;; the built list is borrowed by vec-len then LEAKED (never dropped)
;;
;; LEAK-ONLY: the value is correct (returns n); only the heap accounting is wrong (live-objects grows).
;; FIX: mirror the Proj `reclaim` branch — if heap_operand_ownership(operand)==Owned, stash the operand
;; handle in a scratch slot (tee) before vec-len, then drop it after (vec-len reads a scalar count, so the
;; sequence can be freed immediately). A BORROWED operand (param/kept-let) stays untouched (owner reclaims).
;; Register OP_DROP in the ListLen/BytesLen collect arm. TERRITORY: v-memory-safety (dup/drop placement).
(do
  (def (build (: i Int64) (: n Int64) (: acc (List Int64))) (if (< i n) (build (+ i 1) n (List.push acc i)) acc))
  (def (main (: n Int64)) (List.len (build 0 n (list))))
  (export main))
