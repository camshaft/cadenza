(case "mm1 two SETS built under DIFFERENT handlers then union'd outside both"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def s1 (handle A n
                          ((a (u) s (resume s (+ s 1))))
                          (Set.insert (Set.insert (Set.of (list)) (A.a)) (A.a))))
                (def s2 (handle B 100
                          ((b (u) s (resume s (+ s 10))))
                          (Set.insert (Set.of (list)) (B.b))))
                (def u (Set.union s1 s2))
                (+ (* 100 (Set.len u))
                   (if (Set.contains u 100) 1 0))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 301 Int64)))
