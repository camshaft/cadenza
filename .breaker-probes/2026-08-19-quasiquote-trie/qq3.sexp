(case "qq3 an eval RESULT keys the trie it was computed from (metaprog output re-enters the map)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 5)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def k (eval (quasiquote (+ (unquote 7) 8))))
                (match (Map.lookup m k) ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 75 Int64)))
