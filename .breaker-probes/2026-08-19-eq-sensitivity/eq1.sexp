(case "eq1 deep-trie EQUALITY is content-total: one differing value among 60 breaks ="
  (input  (do
            (def (fill (: i Int64) (: bump Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) bump (Map.insert m i (if (= i 37) (+ (* i 2) bump) (* i 2))))))
            (def (main (: n Int64))
              (do
                (def a (fill n 0 Map.empty))
                (def b (fill n 0 Map.empty))
                (def c (fill n 1 Map.empty))
                (+ (* 10 (if (= a b) 1 0))
                   (if (= a c) 0 1))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 11 Int64)))
