(case "sh2 one RRB list shared as two map VALUES diverges by List.update without cross-talk"
  (input  (do
            (def (build (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (build (- i 1) (List.push acc i))))
            (def (main (: n Int64))
              (do
                (def xs (build n (list)))
                (def m (Map.insert (Map.insert Map.empty 1 xs) 2 xs))
                (def m2 (match (Map.lookup m 1)
                          ((Some l1) (Map.insert m 1 (List.update l1 5 777)))
                          ((None _u) m)))
                (+ (* 10 (match (Map.lookup m2 1)
                           ((Some l1) (match (List.at l1 5) ((Some v) (if (= v 777) 1 0)) ((None _u) -1)))
                           ((None _u) -1)))
                   (match (Map.lookup m2 2)
                     ((Some l2) (match (List.at l2 5) ((Some v) (if (= v 777) 0 1)) ((None _u) -1)))
                     ((None _u) -1)))))
            (export main)))
  (call   main (: 50 Int64)) (output (: 11 Int64)))
