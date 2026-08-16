(case "hx1 Set equality is content-total at depth: one differing element among 60 breaks ="
  (input  (do
            (def (build (: i Int64) (: bump Int64) (: s (Set Int64)))
              (if (= i 0) s (build (- i 1) bump (Set.insert s (if (= i 37) (+ i bump) i)))))
            (def (main (: n Int64))
              (do
                (def a (build n 0 (Set.of (list))))
                (def b (build n 0 (Set.of (list))))
                (def c (build n 1000 (Set.of (list))))
                (+ (* 10 (if (= a b) 1 0))
                   (if (= a c) 0 1))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 11 Int64)))
