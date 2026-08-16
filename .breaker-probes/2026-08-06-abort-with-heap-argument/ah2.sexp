(case "ah2 an abortive arm returns a MAP built FROM its heap op argument as the handle's value"
  (input  (do
            (effect Bail (op stop (-> (List Int64) (Map String Int64))))
            (def (main (: n Int64))
              (let ((m (handle Bail 0
                         ((stop (xs) s (Map.insert Map.empty "sum"
                                          (+ (match (List.at xs 0) ((Some a) a) ((None _u) 0))
                                             (match (List.at xs 1) ((Some b) b) ((None _u) 0))))))
                         (do (Bail.stop (list n 30)) Map.empty))))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m "sum") ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 45 Int64)))
