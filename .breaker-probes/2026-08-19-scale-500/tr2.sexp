(case "tr2 a 500-entry trie builds, enumerates, and rebuilds through tail recursion at scale"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (* i 3) i))))
            (def (rebuild (: ps (List (Tuple Int64 Int64))) (: m (Map Int64 Int64)))
              (match ps
                ((list) m)
                ((list h .. t) (match h ((tuple k v) (rebuild t (Map.insert m k v)))))))
            (def (main (: n Int64))
              (do
                (def src (fill n Map.empty))
                (def rt (rebuild (Map.to-list src) Map.empty))
                (+ (* 10 (if (= rt src) 1 0)) (if (= (Map.len rt) n) 1 0))))
            (export main)))
  (call   main (: 500 Int64)) (output (: 11 Int64)))
