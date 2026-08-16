(case "pm1 REPRO: a TWO-resume-site arm x THREE performs orphans the enclosing param (false CDZ0101)"
  (input  (do
            (effect St (op price (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((price (k) s (if (> k 1) (resume 111 (+ s 1)) (resume 100 s))))
                (+ n (+ (St.price 1) (+ (St.price 7) (St.price 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 327 Int64)))
