(case "nh8 a4-init face: block-wrapped OUTER perform in a nested handle's SEED expression"
  (input  (do
            (effect A (op ga (-> Unit Int64)))
            (effect B (op gb (-> Unit Int64)))
            (def (main)
              (handle A 3 ((ga (u) s (resume s (+ s 1))))
                (handle B (let ((k true)) (if k (A.ga) 9))
                  ((gb (u) t (resume t t)))
                  (+ (* 10 (B.gb)) (A.ga)))))
            (export main)))
  (output (: 34 Int64)))
