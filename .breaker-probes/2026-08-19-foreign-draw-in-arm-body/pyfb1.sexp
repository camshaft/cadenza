(case "pyfb1 probe: A's arm PERFORMS a distinct counter effect B mid-body (before resume), binds it, and uses that foreign draw in BOTH the resume answer and next-state; the B counter advances once per A dispatch"
  (input (do
  (effect A (op tick (-> Int64)))
  (effect B (op beat (-> Int64)))
  (def (main (: n Int64))
    (handle B (: 0 Int64)
      ((beat () bs (resume (+ bs 1) (+ bs 1))))
      (handle A (% n 3)
        ((tick () s
          (let ((k (B.beat)))
            (resume (+ s k) (* s k)))))
        (+ (A.tick) (* 100 (A.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 302 Int64))
  (call   main (: 0 Int64)) (output (: 201 Int64)))
