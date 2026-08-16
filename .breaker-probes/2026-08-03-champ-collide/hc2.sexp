(case "hc2 removing one colliding key collapses the collision node back to the canonical single-key set"
  (input  (do
            (def (main (: z Int64))
              (let ((two (Set.of (list (+ z 0) (+ z 162287980)))))
                (let ((one (Set.remove two 162287981)))
                  (+ (* 100 (if (= one (Set.of (list z))) 1 0))
                     (+ (* 10 (if (Set.contains one 1) 1 0))
                        (if (Set.contains one 162287981) 1 0))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 110 Int64)))
