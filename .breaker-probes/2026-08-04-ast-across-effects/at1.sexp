(case "at1 an Ast node as the RESUME value crosses the handler boundary and is matched by the program"
  (input  (do
            (effect Tmpl (op get (-> Int64 Ast)))
            (def (main (: n Int64))
              (handle Tmpl 0
                ((get (x) s (resume (Ast.Int (BigInt.of x)) s)))
                (match (Tmpl.get n)
                  ((Ast.Int b) b)
                  (_ -1N))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 25 BigInt)))
