(case "pt1 a match over a deep-trie lookup RESULT destructures the retrieved compound at depth"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 (Tuple Int64 String))))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (tuple (* i 2) "tag")))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (match (Map.lookup m 33)
                  ((Some p) (match p ((tuple v s) (+ (* 10 v) (String.byte-len s)))))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 663 Int64)))
