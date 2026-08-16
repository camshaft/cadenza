(case "sc2c dissect: = over two SMALL constant Ast sets"
  (input  (= (Set.of (list (Ast.Int 1N) (Ast.Int 2N))) (Set.of (list (Ast.Int 2N) (Ast.Int 1N)))))
  (output (: true Bool)))
