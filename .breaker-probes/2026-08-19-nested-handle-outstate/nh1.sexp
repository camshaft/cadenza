(case "nh1 adv-69 face 6: block-wrapped OUTER-effect perform in a let-init INSIDE a nested inner handle"
  (input  (do
            (effect A (op ga (-> Unit Int64)))
            (effect B (op gb (-> Unit Int64)))
            (def (main)
              (handle A 3 ((ga (u) s (resume s (+ s 1))))
                (handle B 100 ((gb (u) t (resume t t)))
                  (let ((v (let ((k true)) (if k (A.ga) 9))))
                    (+ (* 10 v) (A.ga))))))
            (export main)))
  (output (: 34 Int64)))
