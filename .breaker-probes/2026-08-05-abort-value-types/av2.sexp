(case "av2 the abort VALUE is an AST node built from the heap state (recursive-sum abort return)"
  (input  (do
            (effect St (op halt (-> Unit Ast)))
            (def (main (: a Int64))
              (match (handle St (list a (+ a 1) (+ a 2))
                       ((halt (u) s (Ast.Int (BigInt.of (List.len s)))))
                       (St.halt))
                ((Ast.Int b) (Int64.of b))
                (_ -1)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 3 Int64)))
