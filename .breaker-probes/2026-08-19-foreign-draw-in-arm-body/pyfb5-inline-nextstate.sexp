(case "pyfb5-inline-nextstate: foreign perform INLINE directly in the tail-arm next-state (multi-dispatch) DECLINES cleanly via the as2 safe-guard — contrast pyfb3 (same perform HOISTED to a let-init, only the binder in next-state) which SLIPS the guard and miscompiles; the inline form is already safe"
  (input (do
  (effect A (op tick (-> Int64)))
  (effect B (op beat (-> Int64)))
  (def (main (: n Int64))
    (handle B (: 0 Int64)
      ((beat () bs (resume (+ bs 1) (+ bs 1))))
      (handle A (% n 3)
        ((tick () s (resume (+ s 1) (+ s (B.beat)))))
        (+ (A.tick) (A.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 5 Int64))
  (call   main (: 0 Int64)) (output (: 3 Int64)))
