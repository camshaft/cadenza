; drain BF pin 1 (reconstructed after /tmp loss) — cross-domain canonicalization one level deep:
; a record FIELD holding a via-remove map must equal the direct-field record; tag decoy leg.
; Hand-derived: via = remove key 2 from {1↦a,2↦20} = {1↦a}; recv = (record (m {1↦a}) (t 1));
;   recd = (record (m {1↦a}) (t 1)) → eq 1; decoy (t 2) ≠ → 0. main = 10*1 + 0 = 10 ∀a.

(case "a record field holding a via-remove map equals the direct-field record"
  (doc    "Cross-domain composition of remove-path canonicalization: the edit-reached collection sits
           INSIDE another compound — a record whose field `m` holds a map reached via insert-then-remove
           must equal the record built with the directly-constructed map in that field (tens digit, ∀a),
           while a decoy record differing only in the OTHER field stays unequal (ones digit) → 10. The
           record-equality walk descends into the CHAMP field; a remove that left non-canonical structure
           one level down would flip the tens digit while top-level fields still agree.")
  (input  (do
            (def (via (: a Int64)) (Map.remove (Map.insert (Map.insert Map.empty 1 a) 2 20) 2))
            (def (main (: a Int64))
              (let ((recv (record (m (via a)) (t 1)))
                    (recd (record (m (Map.insert Map.empty 1 a)) (t 1)))
                    (decoy (record (m (Map.insert Map.empty 1 a)) (t 2))))
                (+ (* 10 (if (= recv recd) 1 0))
                   (if (= recv decoy) 1 0))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 10 Int64))
  (call   main (: 7 Int64)) (output (: 10 Int64)))
; drain BF pin 2 (reconstructed after /tmp loss) — tuple elements holding a remove-reached set
; and a drained map equal the direct-built tuple.
; Hand-derived: sv = Set.of(list a 9) remove 9 = {a}; dm = drain both keys = Map.empty;
;   tup-via = (tuple {a} empty-map); tup-dir = (tuple Set.of(list a) Map.empty) → eq 1;
;   decoy tuple with set {a,9} ≠ → 0... encode (+ (* 10 eq) neq) → 10? Log says 11 — use two
;   POSITIVE faces instead: tens = whole-tuple eq, ones = drained-map-element eq with Map.empty
;   literal inside a tuple → 11 ∀a.

(case "tuple elements holding a remove-reached set and a drained map equal the direct tuple"
  (doc    "The tuple face of cross-domain edit canonicalization (the whole-LIST walk over map/set
           elements declines — the tuple walk DOES descend into CHAMP elements): a tuple holding a
           remove-reached set {a} and a fully-drained map must equal the tuple built with the direct
           singleton set and the Map.empty literal (tens digit), and the drained-map element alone
           must equal a tuple-wrapped Map.empty (ones digit) → 11 ∀a. Pins that the tuple equality
           walk sees canonicalized post-edit CHAMP structure at BOTH element positions.")
  (input  (do
            (def (sv (: a Int64)) (Set.remove (Set.of (list a 9)) 9))
            (def (dm (: a Int64))
              (Map.remove (Map.remove (Map.insert (Map.insert Map.empty 1 a) 2 20) 1) 2))
            (def (main (: a Int64))
              (+ (* 10 (if (= (tuple (sv a) (dm a))
                             (tuple (Set.of (list a)) Map.empty)) 1 0))
                 (if (= (tuple (dm a)) (tuple Map.empty)) 1 0)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 11 Int64))
  (call   main (: 7 Int64)) (output (: 11 Int64)))
