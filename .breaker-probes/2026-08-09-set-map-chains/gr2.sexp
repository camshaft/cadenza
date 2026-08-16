(case "gr2 SET membership of a draw selects a MAP key — two collections chained through one dispatch"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (let ((k (if (Set.contains (Set.of (list 2 5 8)) d) 1 2)))
                    (+ (match (Map.lookup (Map.insert (Map.insert (map) 1 100) 2 200) k)
                         ((Some v) v)
                         ((None) -1))
                       (* 10 (- (E.probe) n)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64))
  (call   main (: 3 Int64)) (output (: 210 Int64))
  (call   main (: 8 Int64)) (output (: 110 Int64)))
