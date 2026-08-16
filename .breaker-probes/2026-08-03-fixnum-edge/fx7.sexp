(case "fx7 the straddling collision pair as Map keys — each retrieves its own value"
  (input  (do
            (def (main (: z Int64))
              (let ((m (Map.insert (Map.insert Map.empty (+ z 134198331) 10) (+ z 536870917) 20)))
                (+ (* 10 (match (Map.lookup m 134198332) ((Some v) (/ v 10)) ((None _u) 0)))
                   (match (Map.lookup m 536870918) ((Some v) (/ v 10)) ((None _u) 0)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 12 Int64)))
