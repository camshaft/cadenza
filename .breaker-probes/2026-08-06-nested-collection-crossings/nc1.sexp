(case "nc1 a LIST OF SETS op result — the body indexes, measures, and probes the nested elements"
  (input  (do
            (effect St (op groups (-> Unit (List (Set Int64)))))
            (def (main (: n Int64))
              (handle St 0
                ((groups (u) s (resume (list (Set.of (list 1 2)) (Set.of (list 3 4 n))) s)))
                (let ((r (St.groups)))
                  (+ (match (List.at r 0) ((Some a) (Set.len a)) ((None _u) -1))
                     (match (List.at r 1) ((Some b) (if (Set.contains b 5) 100 0)) ((None _u) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 102 Int64)))
