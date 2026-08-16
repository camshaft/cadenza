(case "ov1 a trie of OPTION values distinguishes stored-None from absent-key at depth"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 (Option Int64))))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m i (if (= (% i 3) 0) (None unit) (Some (* i 2)))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 1000 (match (Map.lookup m 10) ((Some o) (match o ((Some v) v) ((None _u) -5))) ((None _u) -1)))
                   (+ (* 10 (match (Map.lookup m 9) ((Some o) (match o ((Some _v) 0) ((None _u) 1))) ((None _u) -1)))
                      (match (Map.lookup m 99) ((Some _o) 0) ((None _u) 1))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 20011 Int64)))
