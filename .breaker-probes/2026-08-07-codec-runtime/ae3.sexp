(case "ae3 const control: the codec pin's own shape still folds"
  (input  (match (Ast.decode (Ast.encode (Ast.Int 42N)))
            ((Ok (Ast.Int n)) n)
            (_other 0N)))
  (output (: 42 BigInt)))
