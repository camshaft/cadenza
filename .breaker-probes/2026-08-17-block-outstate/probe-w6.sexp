(case "w6 TWO performs both inside the nested-let-if (order witness)"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((b true)) (if b (+ (* 10 (St.get)) (St.get)) 99))))
                  (+ (* 100 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3405 Int64)))
