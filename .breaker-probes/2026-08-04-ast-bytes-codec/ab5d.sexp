(case "ab5d DISSECT: what does read of a printed Ast.Bytes produce?"
  (input  (do
            (def (main)
              (match (read (print (Ast.Bytes b"hi")))
                ((Ast.Bytes _b) 1)
                ((Ast.Name _s)  2)
                ((Ast.Str _s)   3)
                ((Ast.List _l)  4)
                (_              5)))
            (export main)))
  (output (: 1 Int64)))
