(case "y1 discriminator: perform BEFORE the nested block — does the continuation see seed or pre-block state"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (do
                  (def a (St.get))
                  (def v (let ((b true)) (if b (St.get) 99)))
                  (+ (* 100 a) (+ (* 10 v) (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 345 Int64)))
