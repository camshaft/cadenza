(case "hx2 List equality is content-total at depth: one differing element among 100 breaks ="
  (input  (do
            (def (build (: i Int64) (: bump Int64) (: acc (List Int64)))
              (if (= i 0) acc (build (- i 1) bump (List.push acc (if (= i 63) (+ i bump) i)))))
            (def (main (: n Int64))
              (do
                (def a (build n 0 (list)))
                (def b (build n 0 (list)))
                (def c (build n 5 (list)))
                (+ (* 10 (if (= a b) 1 0))
                   (if (= a c) 0 1))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 11 Int64)))
