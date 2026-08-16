(case "mc1 a MULTI-shot arm re-reduces a continuation that APPLIES a captured closure per resume"
  (input  (do
            (effect Go (op fork (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def scale (fn ((: x Int64)) (* x n)))
                (handle Go 0
                  ((fork (u) s (+ (resume 1 s) (resume 2 s))))
                  (scale (+ (Go.fork) 10)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 115 Int64)))
