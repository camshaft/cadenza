(case "ab1 an ABORTIVE arm discards a 40-key trie state cleanly (no leak, the abort value returns)"
  (input  (do
            (effect Bail (op out (-> Int64 Int64)) (op put (-> Int64 Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) (Bail.out 777) (+ (Bail.put i) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (handle Bail (fill n Map.empty)
                ((put (v) s (resume 0 (Map.insert s (+ v 1000) v)))
                 (out (v) _s v))
                (feed 1 6)))
            (export main)))
  (call   main (: 40 Int64)) (output (: 777 Int64)))
