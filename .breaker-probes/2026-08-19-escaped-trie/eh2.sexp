(case "eh2 the escaped trie is an ordinary value: keyed, churned, and equal to a direct build"
  (input  (do
            (effect Bld (op grow (-> Int64 Int64)) (op take (-> Unit (Map Int64 Int64))))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Bld.grow i) (feed (+ i 1) n))))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 2)))))
            (def (main (: n Int64))
              (do
                (def escaped (handle Bld Map.empty
                               ((grow (v) s (resume 0 (Map.insert s v (* v 2))))
                                (take (u) s (resume s s)))
                               (do
                                 (feed 1 (+ n 1))
                                 (Bld.take))))
                (def direct (fill n Map.empty))
                (+ (* 10 (if (= escaped direct) 1 0))
                   (match (Map.lookup (Map.insert Map.empty direct 42) escaped)
                     ((Some v) (if (= v 42) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 11 Int64)))
