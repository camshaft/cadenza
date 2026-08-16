(case "h2 nested-let-MATCH (not if) wrapping the perform in a let-init"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((k 1)) (match k (1 (St.get)) (_ 99)))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
