(case "ev5 a runtime-woven Ast and its quote twin are ONE Map key (cross-construction-path key identity)"
  (input  (do
            (def (main (: a Int64))
              (match (Map.lookup (Map.insert Map.empty (quote (+ 5 2)) 42)
                                 (Ast.List (list (Ast.Name "+") (Ast.Int (BigInt.of a)) (Ast.Int 2))))
                ((Some v) v) ((None _u) -1)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 42 Int64))
  (call   main (: 6 Int64)) (output (: -1 Int64)))
