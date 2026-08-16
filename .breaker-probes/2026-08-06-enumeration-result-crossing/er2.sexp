(case "er2 Set.to-list of the state crosses resume ORDERED — the body reads elements positionally"
  (input  (do
            (effect St (op dump (-> Unit (List Int64))))
            (def (main (: n Int64))
              (handle St (Set.of (list 30 n 9))
                ((dump (u) s (resume (Set.to-list s) s)))
                (let ((xs (St.dump)))
                  (+ (* 1000 (match (List.at xs 0) ((Some a) a) ((None _u) -1)))
                     (+ (* 10 (match (List.at xs 1) ((Some b) b) ((None _u) -1)))
                        (match (List.at xs 2) ((Some c) c) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5120 Int64)))
