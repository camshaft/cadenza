(case "qq1 a deep-trie lookup result splices into a quasiquote and the eval computes with it"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 5)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def v (match (Map.lookup m 30) ((Some x) x) ((None _u) -1)))
                (eval (quasiquote (+ (unquote v) 8)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 158 Int64)))
