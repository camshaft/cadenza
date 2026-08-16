(case "ab5 print of an Ast.Bytes read back yields an equal node (text round-trip)"
  (input  (= (read (print (Ast.Bytes b"hi"))) (Ast.Bytes b"hi")))
  (output (: true Bool)))
