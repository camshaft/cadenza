(case "rt1 Ok/Err resume values with a DEPENDENT second dispatch — the sign of the falling state flips Ok to Err"
  (input  (do
            (type Res (Ok Int64) (Err Int64))
            (effect E (op run (-> Int64 Res)))
            (def (main (: n Int64))
              (handle E n
                ((run (k) s (resume (if (> s 0) (Ok (* k s)) (Err s)) (- s 2))))
                (match (E.run 3)
                  ((Ok a) (match (E.run 5)
                            ((Ok b) (+ a b))
                            ((Err e2) (+ a (* 1000 e2)))))
                  ((Err e) (* 100 e)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30 Int64))
  (call   main (: 1 Int64)) (output (: -997 Int64))
  (call   main (: -3 Int64)) (output (: -300 Int64)))
