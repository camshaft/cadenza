(case "aq1 Ast.encode bytes re-decoded then structurally compared to a REBUILT tree (decode output vs fresh ctor)"
  (input  (do
            (def (main)
              (match (Ast.decode (Ast.encode (Ast.List (list (Ast.Name "g") (Ast.Bytes b"\x00") (Ast.Int 5N)))))
                ((Ok a) (if (= a (Ast.List (list (Ast.Name "g") (Ast.Bytes b"\x00") (Ast.Int 5N)))) 1 0))
                ((Err _e) -1)))
            (export main)))
  (output (: 1 Int64)))
