(case "pyfb3-nextstate-binder: let-bound EFFECTFUL foreign draw k READ BY the next-state (not just the answer) — v-effects' fix safe-floor DECLINES this bind-once-share case"
  (input (do
  (effect A (op tick (-> Int64)))
  (effect B (op beat (-> Int64)) (op total (-> Int64)))
  (def (main (: n Int64))
    (handle B (: 0 Int64)
      ((beat () bs (resume bs (+ bs 1)))
       (total () bs (resume bs bs)))
      (+ (handle A (% n 3)
           ((tick () s (let ((k (B.beat))) (resume (+ s 1) (+ s k)))))
           (+ (A.tick) (A.tick)))
         (* 10000 (B.total)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 0 Int64)))
