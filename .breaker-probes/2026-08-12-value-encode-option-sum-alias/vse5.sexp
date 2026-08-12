(case "vse5 finding-22 face: Option-sum with Option-INT sibling (not Bytes) — does a scalar sibling alias too?"
  (input  (do
            (type P (A Bytes) (B (Record (: x String))))
            (def (main (: n Int64))
              (record (= payload (Some (P.B (record (= x "hi")))))
                      (= count (: (None unit) (Option Int64)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= count (None unit)) (= payload (Some (B (record (= x "hi")))))) (record (count (Option Int64)) (payload (Option P))))))
