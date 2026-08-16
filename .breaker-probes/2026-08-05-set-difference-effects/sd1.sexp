(case "sd1 Set.difference over handler-built sets (visited MINUS blocked, both built effectfully)"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def visited (handle A n
                               ((a (u) s (resume s (+ s 1))))
                               (Set.insert (Set.insert (Set.insert (Set.of (list)) (A.a)) (A.a)) (A.a))))
                (def blocked (handle A (+ n 1)
                               ((a (u) s (resume s (+ s 1))))
                               (Set.insert (Set.of (list)) (A.a))))
                (def open-set (Set.difference visited blocked))
                (+ (* 10 (Set.len open-set))
                   (if (Set.contains open-set (+ n 1)) 100 (if (Set.contains open-set n) 1 0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 21 Int64)))
