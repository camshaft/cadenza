(case "ao2 control: inner handle RESUMES normally — the outer advance survives"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op ok (-> Unit Int64)))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (let ((b (handle B 0 ((ok (u) s (resume 99 s))) (do (A.tick) (B.ok)))))
                  (+ b (A.get)))))
            (export main)))
  (output (: 110 Int64)))
