(case "rc1 List.update on BOTH sides of a large-RRB merge seam, original concat intact"
  (input  (do
            (def (build (: i Int64) (: n Int64) (: acc (List Int64)))
              (if (= i n) acc (build (+ i 1) n (List.push acc i))))
            (def (main (: k Int64))
              (do
                (def j (List.concat (build 0 600 (list)) (build 600 1200 (list))))
                (def u (List.update (List.update j 599 9001) 600 9002))
                (def (at (: xs (List Int64)) (: i Int64)) (Option.expect (List.at xs i) "in"))
                (+ (* 1000 (if (and (= (at u 599) 9001) (= (at u 600) 9002)) 1 0))
                   (+ (* 100 (if (= (at j 599) 599) 1 0))
                      (+ (* 10 (if (= (at j 600) 600) 1 0))
                         (if (= (List.len u) 1200) 1 0))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1111 Int64)))
