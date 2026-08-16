(case "t3 a try-unwrapped heap list stays live across a SECOND try's failure cut"
  (input  (do
            (def (build (: k Int64))
              (if (> k 0) (Some (list k (+ k 1))) (None unit)))
            (def (grab (: k Int64))
              (let ((inner (try (build k))))
                (let ((more (try (build (- k 1)))))
                  (Some (+ (match (List.at inner 0) ((Some v) v) ((None _u) 0))
                           (* 10 (match (List.at more 1) ((Some v) v) ((None _u) 0))))))))
            (def (main (: k Int64))
              (+ (match (grab k) ((Some v) v) ((None _u) -1))
                 (* 1000 (match (grab 1) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -945 Int64)))
