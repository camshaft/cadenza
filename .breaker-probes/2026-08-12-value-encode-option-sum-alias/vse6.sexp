(case "vse6 finding-22 face: Option-sum whose sum has NO Bytes arm + Option-Bytes sibling"
  (input  (do
            (type Q (C Int64) (D (Record (: x String))))
            (def (main (: n Int64))
              (record (= payload (Some (Q.D (record (= x "hi")))))
                      (= correlation (: (None unit) (Option Bytes)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= correlation (None unit)) (= payload (Some (D (record (= x "hi")))))) (record (correlation (Option Bytes)) (payload (Option Q))))))
