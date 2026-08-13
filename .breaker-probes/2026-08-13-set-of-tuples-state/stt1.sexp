(case "stt1 a SET-OF-TUPLES state seeded by Set.of — structural dedup across dispatches: the repeated pair does not grow the set and the order-swapped pair only counts when the components differ"
  (input  (do
            (effect S (op add (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (Set.of (list (tuple 0 0)))
                ((add (a b) st
                  (let ((s2 (Set.insert st (tuple a b))))
                    (resume (Set.len s2) s2))))
                (let ((a (S.add n 1)))
                  (let ((b (S.add n 1)))
                    (let ((c (S.add 1 n)))
                      (let ((d (S.add (+ n 1) 1)))
                        (+ (* 10 (+ (* 10 (+ (* 10 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2234 Int64))
  (call   main (: 1 Int64)) (output (: 2223 Int64)))
