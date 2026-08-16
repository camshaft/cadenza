(case "ae1 encode-decode round-trip preserves CHAMP key identity for a deep runtime-woven Ast"
  (input  (do
            (def (main (: a Int64))
              (do
                (def tree (Ast.List (list (Ast.Name "+") (Ast.Int (BigInt.of a)) (Ast.List (list (Ast.Name "*") (Ast.Int 2) (Ast.Int 3))))))
                (match (Ast.decode (Ast.encode tree))
                  ((Ok rt) (match (Map.lookup (Map.insert Map.empty tree 42) rt)
                             ((Some v) v) ((None _u) -1)))
                  ((Error _e) -99))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 42 Int64)))
