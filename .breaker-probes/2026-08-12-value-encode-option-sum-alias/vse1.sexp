(case "vse1 a record with SIBLING Option-sum and Option-Bytes fields value-encodes the sum payload intact — the Option-Bytes sibling must not alias its bytes descriptor onto the Option-sum field"
  (input  (do
            (type P (A Bytes) (B (Record (: x String))))
            (def (main (: n Int64))
              (record (= payload (Some (P.B (record (= x "hi")))))
                      (= correlation (: (None unit) (Option Bytes)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= correlation (None unit)) (= payload (Some (B (record (= x "hi")))))) (record (correlation (Option Bytes)) (payload (Option P))))))
