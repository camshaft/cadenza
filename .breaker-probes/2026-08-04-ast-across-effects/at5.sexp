(case "at5 an inner arm performs the OUTER effect with an Ast payload built from the inner op-arg"
  (input  (do
            (effect Outer (op log (-> Ast Int64)))
            (effect Inner (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Outer 0
                ((log (a) s (match a
                              ((Ast.Int b) (resume (Int64.of b) s))
                              (_ (resume -1 s)))))
                (handle Inner 100
                  ((step (v) t (resume (Outer.log (Ast.Int (BigInt.of (+ v t)))) t)))
                  (Inner.step n))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 125 Int64)))
