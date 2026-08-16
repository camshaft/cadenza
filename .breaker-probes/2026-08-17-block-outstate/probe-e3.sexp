(case "e3 result-position if with let-bound cond performing branch"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s s))
                 (put (v) _s (resume unit v)))
                (let ((b (> x 0)))
                  (if b (+ (* 10 (St.get)) x) -1))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 33 Int64)))
