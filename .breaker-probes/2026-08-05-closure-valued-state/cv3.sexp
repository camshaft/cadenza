(case "cv3 the replacement closure CAPTURES the perform-time value (state closes over runtime data)"
  (input  (do
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) x)
                ((eval (v) f (resume (f v) (fn ((: x Int64)) (+ x v)))))
                (+ (* 100 (St.eval n)) (St.eval 3))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 407 Int64)))
