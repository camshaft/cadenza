(case "th3 an expansion Ast is a CHAMP Map key found by its directly-woven twin"
  (input  (do
            (def (mk chunks holes)
              (match holes
                ((list h) (Ast.List (list (Ast.Name "+") h (Ast.Int 2))))
                (_other (Ast.Int 0))))
            (def (main (: a Int64))
              (match (Map.lookup
                       (Map.insert Map.empty (Ast.List (list (Ast.Name "+") (Ast.Int (BigInt.of a)) (Ast.Int 2))) 42)
                       (tagged-template mk (chunks "" "") (holes (Ast.Int (BigInt.of a)))))
                ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 42 Int64)))
