(case "ao4 escalation: TWO committed outer advances before the abort — both lost or both kept?"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Unit Int64)))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (u) s 99)) (do (A.tick) (A.tick) (B.bail)))))
                  (+ b (A.get)))))
            (export main)))
  (output (: 111 Int64)))
