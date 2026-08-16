(case "hn1 a bare BIGINT as op ARGUMENT — the arm does exact wide arithmetic on the crossed box"
  (input  (do
            (effect St (op grow (-> BigInt Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((grow (b) s (resume (Int64.of (/ (* b (BigInt.of 1000000)) (BigInt.of 999999999))) s)))
                (St.grow (BigInt.of (* n 200000)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1000 Int64)))
