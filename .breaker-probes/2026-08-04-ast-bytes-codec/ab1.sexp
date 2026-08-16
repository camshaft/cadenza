(case "ab1 the encoded tag byte of an Ast.Bytes is 0x06"
  (input  (match (Bytes.at (Ast.encode (Ast.Bytes b"")) 0) ((Option.Some b) (Int64.of b)) (_ -1)))
  (output (: 6 Int64)))
