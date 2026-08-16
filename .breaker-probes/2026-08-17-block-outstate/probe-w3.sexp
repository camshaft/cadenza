(case "w3 false-cond else-branch performs same shape"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((b false)) (if b 99 (St.get)))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
