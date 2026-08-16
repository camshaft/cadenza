(case "aq2 a decoded Ast crosses an effect boundary as the RESUME value (decode output feeds the handler path)"
  (input  (do
            (effect Tmpl (op get (-> Unit Ast)))
            (def (main)
              (handle Tmpl 0
                ((get (u) s
                  (match (Ast.decode (Ast.encode (Ast.Int 25N)))
                    ((Ok a)  (resume a s))
                    ((Err _e) (resume (Ast.Name "err") s)))))
                (match (Tmpl.get)
                  ((Ast.Int b) (Int64.of b))
                  (_ -1))))
            (export main)))
  (output (: 25 Int64)))
