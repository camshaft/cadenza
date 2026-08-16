(case "ng1 a trie spanning NEGATIVE and positive keys enumerates in signed numeric order"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (* (- i 50) 7) i))))
            (def (inc (: ps (List (Tuple Int64 Int64))) (: prev Int64) (: cnt Int64))
              (match ps
                ((list) cnt)
                ((list h .. t) (match h ((tuple k _v) (if (> k prev) (inc t k (+ cnt 1)) -100000))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (inc (Map.to-list m) -100000 0))
                   (match (List.at (Map.to-list m) 0)
                     ((Some p) (match p ((tuple k _v) (if (= k -343) 1 0))))
                     ((None _u) -1)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 1001 Int64)))
