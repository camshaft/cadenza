(case "no-op edits (same-value insert, same-value update, remove-of-absent) are canonically equal to the original"
  (doc    "The IDENTITY-edit face of construction-path canonicalization (the AX..CS family pins
           edits that CHANGE the structure; these edits change NOTHING): re-inserting an existing
           key with its EXISTING value (100s), removing an ABSENT key (10s), and List.update writing
           the element's CURRENT value (1s) must each produce a value canonically EQUAL to the
           original → 111 ∀a. A path-copy that rebuilt the spine with any byte difference (a
           freshness counter, a dirtied node header), or a remove-of-absent that restructured on the
           miss path, flips a digit — the persistence-library law (a no-op edit is observationally
           identity) that fmt-idempotency-style fixed-point checks rest on.")
  (input  (do
            (def (main (: a Int64))
              (let ((m (Map.insert (Map.insert Map.empty 1 a) 2 20)))
                (+ (* 100 (if (= (Map.insert m 1 a) m) 1 0))
                   (+ (* 10 (if (= (Map.remove m 99) m) 1 0))
                      (if (= (List.update (list a 2) 0 a) (list a 2)) 1 0)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 111 Int64)))
