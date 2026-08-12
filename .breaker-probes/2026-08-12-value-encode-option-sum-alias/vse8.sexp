(case "vse8 finding-22 face: BARE sum field (not Option-wrapped) + Option-Bytes sibling"
  (input  (do
            (type P (A Bytes) (B (Record (: x String))))
            (def (main (: n Int64))
              (record (= payload (P.B (record (= x "hi"))))
                      (= correlation (: (None unit) (Option Bytes)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= correlation (None unit)) (= payload (B (record (= x "hi"))))) (record (correlation (Option Bytes)) (payload P)))))
