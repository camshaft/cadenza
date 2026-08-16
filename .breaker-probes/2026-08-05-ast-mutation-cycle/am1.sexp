(case "am1 an Ast REWRITE cycle: quote -> transform (add 100 to Ints) -> encode -> decode -> match out"
  (input  (do
            (def (bump (: e Ast))
              (match e
                ((Ast.Int n) (Ast.Int (+ n 100N)))
                ((Ast.List es) (Ast.List (List.push (list) (bump (match (List.at es 0) ((Option.Some x) x) ((Option.None) (Ast.Int 0N)))))))
                (o o)))
            (def (main)
              (match (Ast.decode (Ast.encode (bump (quote 5))))
                ((Ok (Ast.Int n)) (Int64.of n))
                (_ -1)))
            (export main)))
  (output (: 105 Int64)))
