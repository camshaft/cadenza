(case "am1c isolate: encode of a TRANSFORMED node without the fn wrapper (inline match-transform)"
  (input  (do
            (def (main)
              (match (Ast.decode (Ast.encode (match (quote 5) ((Ast.Int n) (Ast.Int (+ n 100N))) (o o))))
                ((Ok (Ast.Int n)) (Int64.of n))
                (_ -1)))
            (export main)))
  (output (: 105 Int64)))
