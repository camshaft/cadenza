(case "no2 the SAME cross-arm perform with the nesting FLIPPED has no home — arm bodies resolve under the handlers enclosing their handle"
  (input  (do
            (effect A (op ga (-> Int64)))
            (effect B (op gb (-> Int64)))
            (def (main (: n Int64))
              (handle B 100
                ((gb () t (resume (+ t (A.ga)) (+ t 10))))
                (handle A n
                  ((ga () s (resume s (+ s 1))))
                  (+ (B.gb) (* 1000 (B.gb))))))
            (export main)))
  (error  CDZ0401))
