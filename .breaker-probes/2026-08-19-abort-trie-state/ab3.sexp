(case "ab3 the abort value is itself a deep-trie READ from the discarded state"
  (input  (do
            (effect Bail (op out (-> Int64 Int64)) (op put (-> Int64 Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 2)))))
            (def (main (: n Int64))
              (handle Bail (fill n Map.empty)
                ((put (v) s (resume 0 (Map.insert s (+ v 1000) v)))
                 (out (v) s (match (Map.lookup s v) ((Some x) x) ((None _v) -1))))
                (do
                  (Bail.put 1)
                  (Bail.out 30))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 60 Int64)))
