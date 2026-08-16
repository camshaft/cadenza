(case "sd2 the difference result becomes the NEXT handle's seed (algebra-into-seed chaining)"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect W (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def visited (handle A n
                               ((a (u) s (resume s (+ s 1))))
                               (Set.insert (Set.insert (Set.of (list)) (A.a)) (A.a))))
                (handle W (Set.difference visited (Set.of (list n)))
                  ((count (u) s (resume (Set.len s) s)))
                  (+ (* 10 (W.count)) (W.count)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))
