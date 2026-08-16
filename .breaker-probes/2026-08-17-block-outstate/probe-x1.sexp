(case "x1 helper FUNCTION containing the if called in the let-init"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (pick (: b Bool)) (if b (St.get) 99))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v (pick true)))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
