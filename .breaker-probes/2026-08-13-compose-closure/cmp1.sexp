(case "cmp1 a COMPOSE-ACCUMULATING closure state — each dispatch replaces the state with a new closure wrapping the OLD one (double-then-add over the previous function), applied at 1 per step"
  (input  (do
            (effect S (op wrap (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (fn ((: x Int64)) (+ x n))
                ((wrap (d) f
                  (let ((f2 (fn ((: x Int64)) (+ (* (f x) 2) d))))
                    (resume (f2 1) f2))))
                (let ((a (S.wrap 0)))
                  (let ((b (S.wrap 5)))
                    (+ (* 1000 a) b)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 8021 Int64))
  (call   main (: 0 Int64)) (output (: 2009 Int64)))
