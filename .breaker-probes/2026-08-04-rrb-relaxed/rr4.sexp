(case "rr4 two relaxed lists with equal content by different concat SHAPES are equal and one Set element"
  (input  (do
            (def (build (: n Int64) (: acc (List Int64)))
              (if (> n 0) (build (- n 1) (List.push acc n)) acc))
            (def (main (: k Int64))
              (let ((a (build 20 (list))) (b (build 30 (list))) (c (build 15 (list))))
                (let ((left (List.concat (List.concat a b) c))
                      (right (List.concat a (List.concat b c))))
                  (+ (* 10 (if (= left right) 1 0))
                     (Set.len (Set.of (list left right)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
