(case "at3 an Ast node as the effect OP ARGUMENT: the arm destructures the performed node"
  (input  (do
            (effect Sink (op eat (-> Ast Int64)))
            (def (main (: n Int64))
              (handle Sink 0
                ((eat (a) s (match a
                              ((Ast.List els) (resume (List.len els) s))
                              (_ (resume -1 s)))))
                (+ (Sink.eat (Ast.List (list (Ast.Int (BigInt.of n)) (Ast.Name "x"))))
                   (Sink.eat (Ast.List (list))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 2 Int64)))
