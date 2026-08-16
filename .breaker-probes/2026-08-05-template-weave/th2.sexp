(case "th2 a tag COMBINES two runtime-woven holes through nested weaving and the result runs deep"
  (input  (do
            (def (nest chunks holes)
              (match holes
                ((list a b) (Ast.List (list (Ast.Name "*") (Ast.List (list (Ast.Name "+") a b)) a)))
                (_other (Ast.Int 0))))
            (def (main (: p Int64) (: q Int64))
              (match (tagged-template nest (chunks "" "" "") (holes (Ast.Int (BigInt.of p)) (Ast.Int (BigInt.of q))))
                ((Ast.List (list (Ast.Name _o) (Ast.List (list (Ast.Name _i) (Ast.Int x) (Ast.Int y))) (Ast.Int z)))
                  (+ (+ x y) z))
                (_other -1N)))
            (export main)))
  (call   main (: 3 Int64) (: 4 Int64))
  (output (: 10 BigInt)))
