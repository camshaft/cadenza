(case "pi1 a PIPE chain threads a trie through three transform stages"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (add-k (: m (Map Int64 Int64)) (: k Int64) (: v Int64)) (Map.insert m k v))
            (def (drop-k (: m (Map Int64 Int64)) (: k Int64)) (Map.remove m k))
            (def (main (: n Int64))
              (do
                (def m (|> (|> (|> (fill n Map.empty) (add-k 999 7)) (drop-k 5)) (add-k 1000 8)))
                (+ (* 100 (Map.len m))
                   (+ (* 10 (match (Map.lookup m 999) ((Some v) v) ((None _u) -1)))
                      (match (Map.lookup m 5) ((Some _v) 0) ((None _u) 1))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 4171 Int64)))
