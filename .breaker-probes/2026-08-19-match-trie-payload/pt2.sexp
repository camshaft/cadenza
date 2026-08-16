(case "pt2 a GUARDED match over a deep-trie lookup gates on the retrieved payload"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 7)))))
            (def (classify (: m (Map Int64 Int64)) (: k Int64))
              (match (Map.lookup m k)
                ((guard (Some v) (> v 200)) 2)
                ((Some _v) 1)
                ((None _u) 0)))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 100 (classify m 35))
                   (+ (* 10 (classify m 10))
                      (classify m 99)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 210 Int64)))
