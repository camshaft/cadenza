(case "gp3 the guard condition calls a PURE HELPER over the state binder — purity analysis admits the def call in guard position"
  (input  (do
            (effect E (op judge (-> Int64 Int64)))
            (def (sq (: t Int64)) (* t t))
            (def (main (: n Int64))
              (handle E n
                ((judge (v) s
                  (match v
                    ((guard x (> x (sq s))) (resume 1 (+ s x)))
                    (_x (resume 0 s)))))
                (+ (* 10 (E.judge 5)) (E.judge 50))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 11 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -4 Int64)) (output (: 1 Int64)))
