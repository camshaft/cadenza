(case "fa1 a FLOAT64 as op ARGUMENT — the arm accumulates fractional values into Float64 state"
  (input  (do
            (effect St (op weigh (-> Float64 Float64)))
            (def (main (: n Int64))
              (handle St 0.5
                ((weigh (x) s (resume (+ x s) (+ s x))))
                (let ((a (St.weigh 1.25)))
                  (let ((b (St.weigh 0.25)))
                    (+ a b)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3.75 Float64)))
