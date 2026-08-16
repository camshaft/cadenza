(case "am1b simpler: quote -> transform -> encode -> decode (no recursion in the transformer)"
  (input  (do
            (def (bump (: e Ast))
              (match e
                ((Ast.Int n) (Ast.Int (+ n 100N)))
                (o o)))
            (def (main)
              (match (Ast.decode (Ast.encode (bump (quote 5))))
                ((Ok (Ast.Int n)) (Int64.of n))
                (_ -1)))
            (export main)))
  (output (: 105 Int64)))
