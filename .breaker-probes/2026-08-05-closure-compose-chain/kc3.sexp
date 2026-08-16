(case "kc3 scalar-capturing replacement (r = f v), THREE performs"
  (input  (do
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) x)
                ((eval (v) f (let ((r (f v))) (resume r (fn ((: x Int64)) (+ x r))))))
                (+ (* 100 (St.eval n)) (+ (* 10 (St.eval 3)) (St.eval 4)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 592 Int64)))
