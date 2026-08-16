(case "ao7 arm-body face: the aborting do inside a MATCH-ARM body (flips on 743fbe231)"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (u) s 99))
                           (match n (1 (do (A.tick) (B.bail))) (_ 7)))))
                  (+ b (A.get)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 110 Int64)))
