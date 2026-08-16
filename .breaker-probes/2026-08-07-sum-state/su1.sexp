(case "su1 a SUM-typed state machine — Idle transitions to Run on first dispatch, Run accumulates thereafter"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (effect M (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle M (Idle)
                ((step (v) s (match s
                               ((Idle) (resume 0 (Run v)))
                               ((Run k) (resume k (Run (+ k v)))))))
                (+ (M.step n) (+ (* 10 (M.step 3)) (* 100 (M.step 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 850 Int64))
  (call   main (: 0 Int64)) (output (: 300 Int64)))
