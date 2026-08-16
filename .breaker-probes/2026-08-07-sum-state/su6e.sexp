(do
  (type Mode (Idle) (Run Int64))
  (effect M (op step (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle M (Idle)
      ((step (v) s (match s
                     ((Idle) (resume 100 (Run (* v 2))))
                     ((Run k) (resume k (Idle))))))
      (+ (M.step 4) (+ (M.step 0) (M.step 9)))))
  (export main))
