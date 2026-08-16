(case "pr1 List.prepend onto a MERGED vector grows the front without disturbing the seam"
  (input  (do
            (def (build (: i Int64) (: n Int64) (: acc (List Int64)))
              (if (= i n) acc (build (+ i 1) n (List.push acc i))))
            (def (main (: k Int64))
              (do
                (def j (List.concat (build 0 40 (list)) (build 40 80 (list))))
                (def g (List.prepend j 999))
                (def (at (: xs (List Int64)) (: i Int64)) (Option.expect (List.at xs i) "in"))
                (+ (* 1000 (List.len g))
                   (+ (* 100 (if (= (at g 0) 999) 1 0))
                      (+ (* 10 (if (= (at g 40) 39) 1 0))
                         (if (= (at g 41) 40) 1 0))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 81111 Int64)))
