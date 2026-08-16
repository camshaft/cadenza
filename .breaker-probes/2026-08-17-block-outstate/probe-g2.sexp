(case "g2 b bound in OUTER let if directly in init"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((b true))
                  (let ((v (if b (St.get) 99)))
                    (+ (* 10 v) (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
