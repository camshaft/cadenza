(case "su5 the sum STATE escapes as the handle's value via a dump op — matched OUTSIDE the handler"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (effect M (op step (-> Int64 Int64)) (op dump (-> Mode)))
            (def (main (: n Int64))
              (match (handle M (Idle)
                       ((step (v) s (match s
                                      ((Idle) (resume 0 (Run v)))
                                      ((Run k) (resume k (Run (+ k v))))))
                        (dump () s (resume s s)))
                       (do (M.step n) (M.step 3) (M.dump)))
                ((Idle) -1)
                ((Run k) (* 2 k))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 16 Int64))
  (call   main (: 0 Int64)) (output (: 6 Int64)))
