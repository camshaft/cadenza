(case "ba2 BigInt division + modulo on a multi-limb state through performs (exact division at scale)"
  (input  (do
            (effect St (op halve (-> Unit Int64)))
            (def (main (: k Int64))
              (handle St 1000000000000000000000000N
                ((halve (u) s (resume (if (= (% s 2N) 0N) 1 0) (/ s 1000000N))))
                (+ (* 100 (St.halve)) (+ (* 10 (St.halve)) (St.halve)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 111 Int64)))
