(case "ao1 v-effects find: inner abort must NOT roll back an outer effect's committed advance"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Unit Int64)))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (u) s 99)) (do (A.tick) (B.bail)))))
                  (+ b (A.get)))))
            (export main)))
  (output (: 110 Int64)))
