(case "d1 two-arm sum match where the FIRST ctor is disc 0 — the eqz leg of the branchless select, both orders"
  (input  (do
            (type Pick (A Int64) (B Int64))
            (def (score (: p Pick))
              (match p
                ((A x) (* 10 x))
                ((B y) (+ 1000 y))))
            (def (main (: n Int64))
              (+ (score (if (= (% n 2) 0) (A n) (B n)))
                 (* 100000 (score (if (= (% n 3) 0) (B n) (A n))))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 100600060 Int64))
  (call   main (: 4 Int64)) (output (: 4000040 Int64))
  (call   main (: 3 Int64)) (output (: 100301003 Int64))
  (call   main (: 5 Int64)) (output (: 5001005 Int64)))
