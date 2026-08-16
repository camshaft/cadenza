(case "kc1 the arm WRAPS the closure state per perform (env chain grows: f then f-then-double)"
  (input  (do
            (effect St (op eval (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (fn ((: x Int64)) x)
                ((eval (v) f (resume (f v) (fn ((: x Int64)) (* (f x) 2)))))
                (+ (* 100 (St.eval n)) (+ (* 10 (St.eval 3)) (St.eval 4)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 576 Int64)))
