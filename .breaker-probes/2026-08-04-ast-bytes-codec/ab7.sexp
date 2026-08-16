(case "ab7 print renders an Ast.Bytes NESTED in an Ast.List with its b-literal spelling"
  (input  (= (print (Ast.List (list (Ast.Name "f") (Ast.Bytes b"\x00\xff"))))
             "(f b\"\\x00\\xff\")"))
  (output (: true Bool)))
