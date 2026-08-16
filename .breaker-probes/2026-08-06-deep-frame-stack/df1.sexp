(case "df1 FOUR nested handler frames, each dispatched once innermost-out"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (effect C (op c (-> Unit Int64)))
            (effect D (op d (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n ((a (u) s (resume s (+ s 1))))
                (handle B 20 ((b (u) s (resume s (+ s 1))))
                  (handle C 300 ((c (u) s (resume s (+ s 1))))
                    (handle D 4000 ((d (u) s (resume s (+ s 1))))
                      (+ (D.d) (+ (C.c) (+ (B.b) (A.a)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4325 Int64)))
