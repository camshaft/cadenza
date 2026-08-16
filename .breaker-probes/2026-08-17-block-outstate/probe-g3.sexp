(case "g3 nested-let around perform-if in MATCH-scrutinee position"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (match (let ((b true)) (if b (St.get) 99))
                  (v (+ (* 10 v) (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
