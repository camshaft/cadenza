(case "be1 BigInt handler state grows PAST i64 range across performs (multi-limb state advance)"
  (input  (do
            (effect Acc (op dbl (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Acc 9223372036854775807N
                ((dbl (u) s (resume (if (> s 9223372036854775807N) 1 0) (+ s s))))
                (+ (Acc.dbl) (+ (Acc.dbl) (Acc.dbl)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2 Int64)))
