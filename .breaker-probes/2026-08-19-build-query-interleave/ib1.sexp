(case "ib1 an INTERLEAVED build-and-query walk: reads against the half-built trie mid-construction"
  (input  (do
            (def (build-q (: i Int64) (: n Int64) (: m (Map Int64 Int64)) (: hits Int64))
              (if (> i n) (tuple m hits)
                (build-q (+ i 1) n (Map.insert m i (* i 2))
                  (+ hits (match (Map.lookup m (- i 5)) ((Some _v) 1) ((None _u) 0))))))
            (def (main (: n Int64))
              (match (build-q 1 n Map.empty 0)
                ((tuple m hits) (+ (* 10 (Map.len m)) hits))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 325 Int64)))
