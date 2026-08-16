(case "pm2 CONTROL: same arm, TWO performs — the param survives"
  (input  (do
            (effect St (op price (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((price (k) s (if (> k 1) (resume 111 (+ s 1)) (resume 100 s))))
                (+ n (+ (St.price 1) (St.price 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 216 Int64)))
