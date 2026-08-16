(case "rc3 UNEVEN merge (37 + 1200): a tiny left against a large right stresses the rebalance"
  (input  (do
            (def (build (: i Int64) (: n Int64) (: acc (List Int64)))
              (if (= i n) acc (build (+ i 1) n (List.push acc i))))
            (def (main (: k Int64))
              (do
                (def j (List.concat (build 0 37 (list)) (build 37 1237 (list))))
                (def (at (: xs (List Int64)) (: i Int64)) (Option.expect (List.at xs i) "in"))
                (+ (* 1000 (if (= (List.len j) 1237) 1 0))
                   (+ (* 100 (if (= (at j 36) 36) 1 0))
                      (+ (* 10 (if (= (at j 37) 37) 1 0))
                         (if (= (at j 1236) 1236) 1 0))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1111 Int64)))
