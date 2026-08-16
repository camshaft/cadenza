(case "mm2 set INTERSECTION of two handler-built sets with a designed overlap"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def s1 (handle A n
                          ((a (u) s (resume s (+ s 1))))
                          (Set.insert (Set.insert (Set.of (list)) (A.a)) (A.a))))
                (def s2 (handle A (+ n 1)
                          ((a (u) s (resume s (+ s 1))))
                          (Set.insert (Set.insert (Set.of (list)) (A.a)) (A.a))))
                (+ (* 10 (Set.len (Set.intersection s1 s2)))
                   (if (Set.contains (Set.intersection s1 s2) (+ n 1)) 1 0))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))
