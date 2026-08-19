(case "pyfb4-single: pyfb3's shape (effectful let-bound k read by next-state) but a SINGLE A dispatch — folds correctly because there's no second dispatch to re-run the let (the distinguisher: multi-dispatch triggers the triangular miscompile, single-dispatch does not; as7-class)"
  (input (do
  (effect A (op tick (-> Int64)))
  (effect B (op beat (-> Int64)))
  (def (main (: n Int64))
    (handle B (: 0 Int64)
      ((beat () bs (resume (+ bs 1) (+ bs 1))))
      (handle A (% n 3)
        ((tick () s (let ((k (B.beat))) (resume (+ s 1) (+ s k)))))
        (+ (A.tick) (: 100 Int64)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 102 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64)))
