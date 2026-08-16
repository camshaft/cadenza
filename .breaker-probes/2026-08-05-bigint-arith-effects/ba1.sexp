(case "ba1 BigInt MULTIPLICATION cascade through performs (multi-limb growth per dispatch)"
  (input  (do
            (effect St (op dbl (-> Unit Int64)))
            (def (main (: k Int64))
              (handle St 1000000000000N
                ((dbl (u) s (resume (if (> s 1000000000000000000000000N) 1 0) (* s 1000000N))))
                (+ (* 100 (St.dbl)) (+ (* 10 (St.dbl)) (St.dbl)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
