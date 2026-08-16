(case "ah1 an abortive arm READS the heap LIST op argument it was handed — the payload survives the abort"
  (input  (do
            (effect Bail (op stop (-> (List Int64) Int64)))
            (def (main (: n Int64))
              (+ 1000
                 (handle Bail 0
                   ((stop (xs) s (+ (* 100 (List.len xs))
                                    (match (List.at xs 1) ((Some v) v) ((None _u) -1)))))
                   (+ 999 (Bail.stop (list n 42 7))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1342 Int64)))
