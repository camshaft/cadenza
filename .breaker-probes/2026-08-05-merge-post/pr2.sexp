(case "pr2 two MERGED vectors with equal content by different split points are equal + one Set element"
  (input  (do
            (def (build (: i Int64) (: n Int64) (: acc (List Int64)))
              (if (= i n) acc (build (+ i 1) n (List.push acc i))))
            (def (main (: k Int64))
              (do
                (def a (List.concat (build 0 25 (list)) (build 25 80 (list))))
                (def b (List.concat (build 0 55 (list)) (build 55 80 (list))))
                (+ (* 10 (if (= a b) 1 0))
                   (Set.len (Set.of (list a b))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
