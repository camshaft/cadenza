(case "ql1 quote/eval over a trie-DRIVEN template: the AST shape itself selected by a lookup"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m i (% i 3)))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def mode (match (Map.lookup m 8) ((Some v) v) ((None _u) -1)))
                (eval (match mode
                        (0 (quote (+ 100 1)))
                        (1 (quote (* 100 2)))
                        (_ (quote (- 100 3)))))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 97 Int64)))
