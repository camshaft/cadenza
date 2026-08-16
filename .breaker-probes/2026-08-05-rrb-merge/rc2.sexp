(case "rc2 concat of two MERGED vectors (concat-of-concats at scale, 2400 elements)"
  (input  (do
            (def (build (: i Int64) (: n Int64) (: acc (List Int64)))
              (if (= i n) acc (build (+ i 1) n (List.push acc i))))
            (def (main (: k Int64))
              (do
                (def j1 (List.concat (build 0 600 (list)) (build 600 1200 (list))))
                (def j2 (List.concat (build 1200 1800 (list)) (build 1800 2400 (list))))
                (def all (List.concat j1 j2))
                (def (at (: xs (List Int64)) (: i Int64)) (Option.expect (List.at xs i) "in"))
                (+ (* 1000 (if (= (List.len all) 2400) 1 0))
                   (+ (* 100 (if (= (at all 1199) 1199) 1 0))
                      (+ (* 10 (if (= (at all 1200) 1200) 1 0))
                         (if (= (at all 2399) 2399) 1 0))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1111 Int64)))
