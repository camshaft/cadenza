(case "nh3 control: DIRECT conditional (no block wrapper) inside the nested handle"
  (input  (do
            (effect A (op ga (-> Unit Int64)))
            (effect B (op gb (-> Unit Int64)))
            (def (main)
              (handle A 3 ((ga (u) s (resume s (+ s 1))))
                (handle B 100 ((gb (u) t (resume t t)))
                  (let ((v (if true (A.ga) 9)))
                    (+ (* 10 v) (A.ga))))))
            (export main)))
  (output (: 34 Int64)))
