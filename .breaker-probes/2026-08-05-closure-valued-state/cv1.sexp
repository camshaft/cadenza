(case "cv1 the handler STATE is a CLOSURE the arm applies per perform (strategy-as-state)"
  (input  (do
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) (* x 2))
                ((eval (v) f (resume (f v) f)))
                (+ (* 100 (St.eval n)) (St.eval 3))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 806 Int64)))
