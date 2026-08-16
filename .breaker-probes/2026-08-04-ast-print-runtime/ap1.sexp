(case "ap1 print of a RUNTIME-BUILT Ast tree renders (not constant-only): wrap depth 3"
  (input  (do
            (def (wrap (: i Int64) (: node Ast))
              (if (= i 0) node (wrap (- i 1) (Ast.List (list (Ast.Name "f") node)))))
            (def (main (: n Int64))
              (String.scalar-len (print (wrap n (Ast.Int 5N)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 13 Int64)))
