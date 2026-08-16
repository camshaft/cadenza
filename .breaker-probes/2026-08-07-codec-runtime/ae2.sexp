(case "ae2 minimal: decode result destructured Ok-only + wildcard"
  (input  (do
            (def (main (: a Int64))
              (match (Ast.decode (Ast.encode (Ast.Int (BigInt.of a))))
                ((Ok (Ast.Int n)) (Int64.of n))
                (_other -1)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
