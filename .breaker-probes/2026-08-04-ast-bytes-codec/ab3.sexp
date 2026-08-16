(case "ab3 a quote-built and constructor-built BYTES AST encode to identical bytes"
  (input  (= (Ast.encode (quote b"hi")) (Ast.encode (Ast.Bytes b"hi"))))
  (output (: true Bool)))
