(case "rb1 read of a name that STARTS with b but isn't a byte literal: (read \"bx\") is an Ast.Name"
  (input  (match (read "bx") ((Ast.Name _n) 1) ((Ast.Bytes _b) 2) (_ 3)))
  (output (: 1 Int64)))
