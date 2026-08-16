(case "wv2 the served two-site arm nested one handler DEEPER (does the refold hold under a live outer frame?)"
  (input  (do
            (effect Out (op peek (-> Unit Int64)))
            (effect St (op price (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Out 500
                ((peek (u) w (resume w w)))
                (+ (Out.peek)
                   (handle St 0
                     ((price (k) s (if (> k 1) (resume 111 (+ s 1)) (resume 100 s))))
                     (+ (St.price 1) (+ (St.price n) (St.price 2)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 822 Int64)))
