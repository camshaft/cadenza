(case "sh2x a closure captures the OUTER binding while the shadow is live inside"
  (input  (do
            (def (main (: n Int64))
              (do
                (def k n)
                (def f (fn ((: x Int64)) (+ x k)))
                (def inner
                  (let ((k 1000))
                    (+ k (f 1))))
                (+ inner (f 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1013 Int64)))
