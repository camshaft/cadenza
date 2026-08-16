(case "cc2 a closure escaping handle A is applied inside handle B — the A-capture is stable while B's draws feed the args"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (main (: n Int64))
              (let ((f (handle A n
                         ((a () s (resume s (+ s 1))))
                         (let ((k (A.a))) (fn ((: x Int64)) (+ x k))))))
                (handle B 100
                  ((b () t (resume t (* t 2))))
                  (+ (f (B.b)) (f (B.b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 310 Int64))
  (call   main (: 0 Int64)) (output (: 300 Int64)))
