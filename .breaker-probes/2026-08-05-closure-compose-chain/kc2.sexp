(case "kc2 wrap-composing closure state, TWO performs"
  (input  (do
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) x)
                ((eval (v) f (resume (f v) (fn ((: x Int64)) (* (f x) 2)))))
                (+ (* 10 (St.eval n)) (St.eval 3))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
