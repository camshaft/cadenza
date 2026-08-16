(case "pt1 print∘weave = the source spelling for a runtime-woven tree (render canonicality)"
  (input  (do
            (def (main (: a Int64))
              (do
                (def tree (Ast.List (list (Ast.Name "+") (Ast.Int (BigInt.of a)) (Ast.Int 2))))
                (+ (* 10 (if (= (print tree) "(+ 5 2)") 1 0))
                   (if (= (print tree) (print (quote (+ 5 2)))) 1 0))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))
