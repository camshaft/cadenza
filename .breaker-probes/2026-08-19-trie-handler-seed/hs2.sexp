(case "hs2 an arm REPLACES the trie state wholesale and the next op reads the replacement"
  (input  (do
            (effect Sw (op swap (-> Unit Int64)) (op len (-> Unit Int64)))
            (def (fill (: i Int64) (: k Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) k (Map.insert m (+ (* k 1000) i) i))))
            (def (main (: n Int64))
              (handle Sw (fill n 1 Map.empty)
                ((swap (u) s (resume (Map.len s) (fill (* n 2) 2 Map.empty)))
                 (len (u) s (resume (Map.len s) s)))
                (+ (* 1000 (Sw.swap)) (Sw.len))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 30060 Int64)))
