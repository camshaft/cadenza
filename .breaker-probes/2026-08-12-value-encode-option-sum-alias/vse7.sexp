(case "vse7 finding-22 face: TWO Option-sum siblings — which way does the alias fall?"
  (input  (do
            (type P (A Bytes) (B (Record (: x String))))
            (type Q (C Int64) (D (Record (: y Int64))))
            (def (main (: n Int64))
              (record (= first (Some (P.B (record (= x "hi")))))
                      (= second (Some (Q.D (record (= y 7)))))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= first (Some (B (record (= x "hi"))))) (= second (Some (D (record (= y 7)))))) (record (first (Option P)) (second (Option Q))))))
