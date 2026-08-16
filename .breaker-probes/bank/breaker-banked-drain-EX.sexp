(case "two stateful handlers with HEAP states interleave performs without cross-contamination"
  (doc    "The HEAP-state upgrade of the two-handler independence pin (:6441's two states are
           scalars): the outer handler threads a LIST, the inner a SET, and four performs interleave
           L-S-L-S — each resume reads the CURRENT length of ITS OWN state before growing it (L: 0
           then 1; S: 0 then 1 — the second S.add inserts a DUPLICATE, so the set's dedup is also
           live in the state thread) → 11. Two heap state slots threading through interleaved frames
           is the cross-contamination face: a state-slot confusion between the handlers would read
           the other collection's length (list len where set len belongs), and a dropped heap state
           re-seed shows 0s everywhere.")
  (input  (do
            (effect L (op push (-> Int64 Int64)))
            (effect S (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle L (list)
                ((push (v) ls (resume (List.len ls) (List.push ls v))))
                (handle S (Set.of (list))
                  ((add (v) ss (resume (Set.len ss) (Set.insert ss v))))
                  (+ (* 1000 (L.push 10))
                     (+ (* 100 (S.add 7))
                        (+ (* 10 (L.push 20))
                           (S.add 7)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
