(case "a MAP-of-MAPS two-level lookup updates the inner map through a keyed RMW"
  (doc    "The two-level READ-MODIFY-WRITE (the shared-inner pin :2456 covers Perceus refcounts when
           generations share an inner; this pins the UPDATE ROUTE): lookup outer key 1, insert into
           the INNER map, re-insert at the outer key — the new entry reads through both levels
           (10000s: m2[1][k]=999), the outer SIBLING entry's inner is untouched (100s: len 1), and
           the ORIGINAL outer's inner still has len 1 (1s: the RMW path-copied both levels, no
           in-place write observed through the old binding) → 10101. The nested-namespace idiom
           (config sections, per-tenant tables): outer path-copy + inner path-copy composed in one
           expression.")
  (input  (do
            (def (main (: k Int64))
              (let ((m (Map.insert (Map.insert Map.empty
                          1 (Map.insert Map.empty 10 100))
                          2 (Map.insert Map.empty 20 200))))
                (let ((m2 (match (Map.lookup m 1)
                            ((Some inner) (Map.insert m 1 (Map.insert inner k 999)))
                            ((None u) m))))
                  (+ (* 10000 (match (Map.lookup m2 1)
                                ((Some i2) (match (Map.lookup i2 k) ((Some v) (- v 998)) ((None u2) -1)))
                                ((None u3) -1)))
                     (+ (* 100 (match (Map.lookup m2 2)
                                 ((Some i3) (Map.len i3)) ((None u4) -1)))
                        (match (Map.lookup m 1)
                          ((Some i4) (Map.len i4)) ((None u5) -1)))))))
            (export main)))
  (call   main (: 11 Int64)) (output (: 10101 Int64)))
