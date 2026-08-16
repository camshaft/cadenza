(case "fe6 a Float64 TUPLE state — one slot advances additively, the other doubles, both exact across two dispatches"
  (input  (do
            (effect E (op pair (-> (Tuple Float64 Float64))))
            (def (main (: u Float64))
              (handle E (tuple 0.5 1.0)
                ((pair () s (match s
                              ((tuple a b) (resume s (tuple (+ a 1.5) (* b 2.0)))))))
                (match (E.pair)
                  ((tuple a1 b1)
                    (match (E.pair)
                      ((tuple a2 b2) (+ (* 100.0 a1) (+ (* 10.0 b1) (+ a2 b2)))))))))
            (export main)))
  (call   main (: 0.0 Float64)) (output (: 64.0 Float64)))
