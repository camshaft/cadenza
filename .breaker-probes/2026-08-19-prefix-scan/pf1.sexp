(case "pf1 a PREFIX-SUM rebuild: running totals materialize as a parallel trie"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (% (* i 13) 10)))))
            (def (scan (: ps (List (Tuple Int64 Int64))) (: run Int64) (: out (Map Int64 Int64)))
              (match ps
                ((list) out)
                ((list h .. t) (match h ((tuple k v)
                  (scan t (+ run v) (Map.insert out k (+ run v))))))))
            (def (main (: n Int64))
              (do
                (def src (fill n Map.empty))
                (def sums (scan (Map.to-list src) 0 Map.empty))
                (+ (* 10 (match (Map.lookup sums n) ((Some total) total) ((None _u) -1)))
                   (match (Map.lookup sums 10) ((Some p) (% p 10)) ((None _u) -1)))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 1155 Int64)))
