(case "g1 cond derived from nested-let int"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((k 1)) (if (> k 0) (St.get) 99))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
