(case "ab10 eval of a template splicing an Ast.Bytes VALUE declines — the ruled Ast-value-splice boundary reaches Bytes"
  (input  (eval (quasiquote (Bytes.concat (unquote (Ast.Bytes b"hi")) b"!"))))
  (declines))
