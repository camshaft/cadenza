(case "pyfb3-valonly: let-bound EFFECTFUL foreign draw k read ONLY by the resume VALUE (next-state pure) — the do-peel/let-peel value-position case that FOLDS (k runs once/dispatch), contrast pyfb3 where k feeds next-state and still miscompiles"
  (input (do
  (effect A (op tick (-> Int64)))
  (effect B (op beat (-> Int64)))
  (def (main (: n Int64))
    (handle B (: 0 Int64)
      ((beat () bs (resume (+ bs 1) (+ bs 1))))
      (handle A (% n 3)
        ((tick () s (let ((k (B.beat))) (resume (+ s k) (+ s 1)))))
        (+ (A.tick) (* 100 (A.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 402 Int64))
  (call   main (: 0 Int64)) (output (: 301 Int64)))
