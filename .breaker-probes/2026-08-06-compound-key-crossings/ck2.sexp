(case "ck2 a SET of tuples as op ARGUMENT — the arm probes compound membership including order sensitivity"
  (input  (do
            (effect St (op check (-> (Set (Tuple Int64 Int64)) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((check (xs) s
                  (resume (+ (* 100 (if (Set.contains xs (tuple 1 n)) 1 0))
                             (+ (* 10 (if (Set.contains xs (tuple n 1)) 1 0))
                                (Set.len xs)))
                          s)))
                (St.check (Set.of (list (tuple 1 n) (tuple 2 8))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 102 Int64)))
