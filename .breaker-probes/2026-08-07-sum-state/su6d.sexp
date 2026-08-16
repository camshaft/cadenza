(do
  (type Mode (Idle) (Run Int64))
  (effect M (op step (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle M (Run n)
      ((step (v) s (match s
                     ((Idle) (resume 0 (Run v)))
                     ((Run k) (resume k (Run (+ k v)))))))
      (+ (M.step 1)
         (+ (* 10 (handle M (Idle)
                    ((step (v) s (match s
                                   ((Idle) (resume 100 (Run (* v 2))))
                                   ((Run k) (resume k (Idle))))))
                    (+ (M.step 4) (M.step 0))))
            (* 1000 (M.step 2))))))
  (export main))
