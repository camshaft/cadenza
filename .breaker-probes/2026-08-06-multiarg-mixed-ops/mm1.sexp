(case "mm1 a MULTI-argument op mixing a heap list and two scalars — the arm consumes all three"
  (input  (do
            (effect St (op pick (-> (List Int64) Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((pick (xs lo hi) s
                  (resume (+ (* 100 (match (List.at xs lo) ((Some a) a) ((None _u) -1)))
                             (+ (* 10 (match (List.at xs hi) ((Some b) b) ((None _u) -1)))
                                (List.len xs)))
                          s)))
                (St.pick (list 7 n 9) 0 2)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 793 Int64)))
