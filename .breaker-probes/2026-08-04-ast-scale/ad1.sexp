(case "ad1 a runtime-built 200-deep Ast spine walks by head-recursion without stack or rep failure"
  (input  (do
            (def (wrap (: i Int64) (: node Ast))
              (if (= i 0) node (wrap (- i 1) (Ast.List (list node (Ast.Int 7N))))))
            (def (depth (: node Ast))
              (match node
                ((Ast.List es) (match es ((list) 1) ((list h .. _) (+ 1 (depth h)))))
                (_ 1)))
            (def (main (: n Int64))
              (depth (wrap n (Ast.Int 5N))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 201 Int64)))
