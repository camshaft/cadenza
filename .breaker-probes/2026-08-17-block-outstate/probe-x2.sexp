(case "x2 nested-let-if init consumed by MATCH binder instead of let"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (do
                  (def v (let ((b true)) (if b (St.get) 99)))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
