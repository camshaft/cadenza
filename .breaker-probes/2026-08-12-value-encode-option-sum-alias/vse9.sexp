(case "vse9 finding-22 face: field NAMES reversed so the SUM field sorts FIRST — does the alias flip direction?"
  (input  (do
            (type P (A Bytes) (B (Record (: x String))))
            (def (main (: n Int64))
              (record (= apayload (Some (P.B (record (= x "hi")))))
                      (= zcorrelation (: (None unit) (Option Bytes)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= apayload (Some (B (record (= x "hi"))))) (= zcorrelation (None unit))) (record (apayload (Option P)) (zcorrelation (Option Bytes))))))
