(case "gp1 a match GUARD inside the handler ARM compares the op PAYLOAD against the live STATE binder — the guard routes admit/reject per dispatch"
  (input  (do
            (effect E (op judge (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((judge (v) s
                  (match v
                    ((guard x (> x s)) (resume 1 (+ s x)))
                    (_x (resume 0 s)))))
                (+ (* 10 (E.judge 3)) (E.judge 4))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 0 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -3 Int64)) (output (: 11 Int64)))
