(case "ad2 a 200-deep runtime-built Ast spine ENCODES and DECODES back to a structurally equal tree"
  (input  (do
            (def (wrap (: i Int64) (: node Ast))
              (if (= i 0) node (wrap (- i 1) (Ast.List (list node (Ast.Int 7N))))))
            (def (main (: n Int64))
              (do
                (def t (wrap n (Ast.Int 5N)))
                (match (Ast.decode (Ast.encode t))
                  ((Ok a)  (if (= a t) 1 0))
                  ((Err _e) -1))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 1 Int64)))
