(case "ea2 a handler's heap STATE grows to a 40-key trie across resumes and enumerates at the end"
  (input  (do
            (effect Acc (op put (-> Int64 Int64)) (op total (-> Unit Int64)))
            (def (sum-pairs (: ps (List (Tuple Int64 Int64))) (: acc Int64))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple _k v) (sum-pairs t (+ acc v)))))))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Acc.put i) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (handle Acc Map.empty
                ((put (v) s (resume 0 (Map.insert s v (* v 10))))
                 (total (u) s (resume (sum-pairs (Map.to-list s) 0) s)))
                (do
                  (feed 1 (+ n 1))
                  (Acc.total))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 8200 Int64)))
