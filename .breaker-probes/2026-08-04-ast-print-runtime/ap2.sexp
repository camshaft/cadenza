(case "ap2 control: print of a CONSTANT ctor-built nested tree"
  (input  (String.scalar-len (print (Ast.List (list (Ast.Name "f") (Ast.Int 5N))))))
  (output (: 5 Int64)))
