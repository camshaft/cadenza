(case "ab4 the Ast.Bytes encoding is exactly tag + 4-byte LE length + raw payload"
  (input  (Bytes.len (Ast.encode (Ast.Bytes b"hi"))))
  (output (: 7 Int64)))
