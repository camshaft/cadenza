(case "ae1 Ast.encode outputs key a Bytes-trie at depth; a quote-built probe hits the ctor-built entry"
  (input  (do
            (def (fill (: i Int64) (: m (Map Bytes Int64)))
              (if (= i 0) m (fill (- i 1) (Map.insert m (Ast.encode (Ast.Int (BigInt.of i))) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (Ast.encode (quote 25))) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 425 Int64)))
