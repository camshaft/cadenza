(case "su6f a SCALAR-state outer shadowed by a SUM-state inner cycler — the inner machine transitions twice"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (effect M (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle M n
                ((step (v) s (resume s (+ s v))))
                (+ (M.step 1)
                   (* 10 (handle M (Idle)
                           ((step (v) s (match s
                                          ((Idle) (resume 100 (Run (* v 2))))
                                          ((Run k) (resume k (Idle))))))
                           (+ (M.step 4) (M.step 0)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1085 Int64))
  (call   main (: 0 Int64)) (output (: 1080 Int64)))
