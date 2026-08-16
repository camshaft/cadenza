(case "pt2 minimal runtime print"
  (input  (do
            (def (main (: a Int64))
              (String.byte-len (print (Ast.Int (BigInt.of a)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
