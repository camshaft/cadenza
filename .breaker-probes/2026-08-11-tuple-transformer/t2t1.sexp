(case "t2t1 a TUPLE-to-TUPLE transformer op chained twice — the arm swaps components and salts with the state, both crossings exact"
  (input  (do
            (effect S (op swap2 (-> (Tuple Int64 Int64) (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle S 0
                ((swap2 (p) s
                  (match p ((tuple a b) (resume (tuple (+ b s) a) (+ s 1))))))
                (match (S.swap2 (tuple n 20))
                  ((tuple x y)
                    (match (S.swap2 (tuple x y))
                      ((tuple u v) (+ (* 1000 u) v)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 4020 Int64))
  (call   main (: 0 Int64)) (output (: 1020 Int64)))
