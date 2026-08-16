(case "ql4 the WORKING side: eval of a DIRECT quote with a spliced trie value"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (* i 4)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def v (match (Map.lookup m 6) ((Some x) x) ((None _u) -1)))
                (eval (quasiquote (* (unquote v) 2)))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 48 Int64)))
