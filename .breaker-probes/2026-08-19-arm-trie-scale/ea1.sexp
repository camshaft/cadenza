(case "ea1 a handler ARM enumerates a 60-key trie carried as the op argument and resumes its fold"
  (input  (do
            (effect Sink (op tally (-> (Map Int64 Int64) Int64)))
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
            (def (sum-pairs (: ps (List (Tuple Int64 Int64))) (: acc Int64))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple _k v) (sum-pairs t (+ acc v)))))))
            (def (main (: n Int64))
              (handle Sink 0
                ((tally (m) s (resume (sum-pairs (Map.to-list m) 0) s)))
                (Sink.tally (fill n Map.empty))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 1830 Int64)))
