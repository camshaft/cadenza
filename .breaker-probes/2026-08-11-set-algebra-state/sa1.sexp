(case "sga1 the arm answers with SET ALGEBRA over its state and an argument-built set — union, intersection, and difference sizes cross dispatch"
  (input  (do
            (effect S (op probe (-> Int64 Int64)) (op grow (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (Set.of (list n (+ n 2)))
                ((probe (v) s
                  (let ((arg (Set.of (list v (+ v 1)))))
                    (resume (+ (* 100 (Set.len (Set.union s arg)))
                               (+ (* 10 (Set.len (Set.intersection s arg)))
                                  (Set.len (Set.difference s arg))))
                            s)))
                 (grow (v) s (resume (Set.len s) (Set.insert s v))))
                (let ((a (S.probe n)))
                  (let ((b (S.grow (+ n 1))))
                    (let ((c (S.probe n)))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3110521 Int64))
  (call   main (: 0 Int64)) (output (: 3110521 Int64)))
