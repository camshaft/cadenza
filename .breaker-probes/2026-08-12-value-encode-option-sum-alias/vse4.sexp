(case "vse4 finding-22 face: the sibling record is built IN AN ARM and crosses resume before being returned"
  (input  (do
            (type P (A Bytes) (B (Record (: x String))))
            (effect S (op mk (-> (Record (: payload (Option P)) (: correlation (Option Bytes))))))
            (def (main (: n Int64))
              (handle S n
                ((mk () s
                  (resume (record (= payload (Some (P.B (record (= x "hi")))))
                                  (= correlation (: (None unit) (Option Bytes))))
                          s)))
                (S.mk)))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= correlation (None unit)) (= payload (Some (B (record (= x "hi")))))) (record (correlation (Option Bytes)) (payload (Option P))))))
