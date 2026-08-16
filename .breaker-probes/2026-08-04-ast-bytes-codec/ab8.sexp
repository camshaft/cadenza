(case "ab8 an unquote splices a computed Ast.Bytes node into a quasiquote template"
  (input  (do
            (def (main)
              (match (quasiquote (f (unquote (Ast.Bytes b"hi"))))
                ((Ast.List els)
                  (match (List.at els 1)
                    ((Option.Some (Ast.Bytes b)) (Bytes.len b))
                    (_ -2)))
                (_ -1)))
            (export main)))
  (output (: 2 Int64)))
