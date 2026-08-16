(case "rsw1 the arm's Ok/Err verdict FLIPS between two identical performs as the state passes the payload — both variants cross and unwrap"
  (input  (do
            (effect S (op step (-> Int64 (Result Int64 Int64))))
            (def (unwrap-or (: r (Result Int64 Int64)) (: d Int64))
              (match r ((Ok v) v) ((Err e) (+ d e))))
            (def (main (: n Int64))
              (handle S n
                ((step (v) s
                  (resume (if (< v s) (Ok (* v 10)) (Err (- v s))) (+ s 1))))
                (+ (* 1000 (unwrap-or (S.step 3) -100))
                   (unwrap-or (S.step 3) -100))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30030 Int64))
  (call   main (: 2 Int64)) (output (: -99100 Int64))
  (call   main (: 3 Int64)) (output (: -99970 Int64)))
