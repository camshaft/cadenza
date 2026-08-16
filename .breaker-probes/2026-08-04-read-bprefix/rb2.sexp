(case "rb2 read of b\"\" (empty byte literal text) is an EMPTY Ast.Bytes, not a name"
  (input  (match (read "b\"\"") ((Ast.Bytes b) (Bytes.len b)) ((Ast.Name _n) -1) (_ -2)))
  (output (: 0 Int64)))
