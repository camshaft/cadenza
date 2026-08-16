(case "dq1 a DEQUE discipline over RRB: push both ends via concat, read both ends at depth"
  (input  (do
            (def (build (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc
                (build (- i 1)
                  (if (= (% i 2) 0)
                      (List.concat acc (list i))
                      (List.concat (list i) acc)))))
            (def (main (: n Int64))
              (do
                (def xs (build n (list)))
                (+ (* 1000 (match (List.at xs 0) ((Some v) v) ((None _u) -1)))
                   (+ (List.len xs)
                      (match (List.at xs (- (List.len xs) 1)) ((Some v) v) ((None _u) -1))))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 1062 Int64)))
