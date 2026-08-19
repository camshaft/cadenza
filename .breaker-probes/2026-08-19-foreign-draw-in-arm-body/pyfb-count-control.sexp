(case "pyfb-ctrl-count: arm performs B.beat (counted) then resumes with pure values; count arm entries for (+ (A.tick)(A.tick))"
  (input (do
  (effect A (op tick (-> Int64)))
  (effect B (op beat (-> Int64)) (op total (-> Int64)))
  (def (main (: n Int64))
    (handle B (: 0 Int64)
      ((beat () bs (resume bs (+ bs 1)))
       (total () bs (resume bs bs)))
      (+ (handle A (% n 3)
           ((tick () s
             (do (B.beat)
                 (resume (+ s 1) (+ s 1)))))
           (+ (A.tick) (A.tick)))
         (* 10000 (B.total)))))
  (export main)))
  (call main (: 10 Int64)) (output (: 0 Int64)))
