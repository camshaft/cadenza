(case "nc2 a LIST OF SETS as op ARGUMENT — the arm indexes into the nested payload it is handed"
  (input  (do
            (effect St (op weigh (-> (List (Set Int64)) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((weigh (xs) s
                  (resume (+ (match (List.at xs 0) ((Some a) (+ (* 10 (Set.len a)) (if (Set.contains a 5) 100 0))) ((None _u) -1))
                             (match (List.at xs 1) ((Some b) (Set.len b)) ((None _u) -1)))
                          s)))
                (St.weigh (list (Set.of (list n 2)) (Set.of (list 7))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 121 Int64)))
