(case "ng1 NEGATIVE operands through modulo/division in arm arithmetic (sign semantics in the state slot)"
  (input  (do
            (effect St (op step (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St -7
                ((step (u) s (resume (% s 3) (/ s 2))))
                (+ (* 100 (St.step)) (+ (* 10 (St.step)) (St.step)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: -101 Int64)))
