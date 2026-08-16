(case "tl1 Map.to-list over a MULTI-LEVEL trie enumerates all 100 keys fully sorted"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (* i 7) i))))
            (def (sorted (: ps (List (Tuple Int64 Int64))) (: prev Int64) (: cnt Int64))
              (match ps
                ((list) cnt)
                ((list h .. t) (match h ((tuple k _v) (if (> k prev) (sorted t k (+ cnt 1)) -100000))))))
            (def (main (: n Int64))
              (sorted (Map.to-list (fill n Map.empty)) -1 0))
            (export main)))
  (call   main (: 100 Int64)) (output (: 100 Int64)))
