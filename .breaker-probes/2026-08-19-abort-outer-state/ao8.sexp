(case "ao8 DEFERRED: performing if-CONDITION before the branch-abort (stays 109 until the cv-lift)"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op bail (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (let ((b (handle B 0 ((bail (u) s 99))
                           (if (> (A.tick) 5) (B.bail) (B.bail)))))
                  (+ b (A.get)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 110 Int64)))
