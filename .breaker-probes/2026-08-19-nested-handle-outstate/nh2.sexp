(case "nh2 control: same shape WITHOUT the nested inner handle (floor territory)"
  (input  (do
            (effect A (op ga (-> Unit Int64)))
            (def (main)
              (handle A 3 ((ga (u) s (resume s (+ s 1))))
                (let ((v (let ((k true)) (if k (A.ga) 9))))
                  (+ (* 10 v) (A.ga)))))
            (export main)))
  (output (: 34 Int64)))
