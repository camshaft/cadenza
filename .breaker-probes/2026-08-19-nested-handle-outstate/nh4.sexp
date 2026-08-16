(case "nh4 twin: block-wrapped INNER-effect perform in the inner handle's own let-init"
  (input  (do
            (effect A (op ga (-> Unit Int64)))
            (effect B (op gb (-> Unit Int64)))
            (def (main)
              (handle A 3 ((ga (u) s (resume s (+ s 1))))
                (handle B 100 ((gb (u) t (resume t (+ t 1))))
                  (let ((v (let ((k true)) (if k (B.gb) 9))))
                    (+ (* 10 v) (B.gb))))))
            (export main)))
  (output (: 1101 Int64)))
