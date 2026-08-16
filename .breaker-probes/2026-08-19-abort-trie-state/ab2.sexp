(case "ab2 simpler: abortive arm discards a trie state grown by prior resumes"
  (input  (do
            (effect Bail (op out (-> Int64 Int64)) (op put (-> Int64 Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (main (: n Int64))
              (handle Bail (fill n Map.empty)
                ((put (v) s (resume 0 (Map.insert s (+ v 1000) v)))
                 (out (v) _s v))
                (do
                  (Bail.put 1)
                  (Bail.put 2)
                  (Bail.out 777))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 777 Int64)))
