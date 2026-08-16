(case "su6g a SUM-state outer shadowed by a SCALAR-state inner — mixed state kinds across the shadow boundary"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (effect M (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle M (Run n)
                ((step (v) s (match s
                               ((Idle) (resume 0 (Run v)))
                               ((Run k) (resume k (Run (+ k v)))))))
                (+ (M.step 1)
                   (* 10 (handle M 0
                           ((step (v) t (resume (+ t v) (+ t 1))))
                           (+ (M.step 4) (M.step 0)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 55 Int64))
  (call   main (: 0 Int64)) (output (: 50 Int64)))
