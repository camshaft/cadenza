(case "pm3 CONTROL: single-site arm, THREE performs, param read — survives"
  (input  (do
            (effect St (op price (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((price (k) s (resume (* k 10) (+ s 1))))
                (+ n (+ (St.price 1) (+ (St.price 7) (St.price 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))
