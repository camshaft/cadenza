(case "at2 an Ast.List handler STATE accumulates a node per perform and survives the round-trips"
  (input  (do
            (effect Acc (op put (-> Int64 Int64)))
            (def (main (: a Int64) (: b Int64))
              (handle Acc (Ast.List (list))
                ((put (v) s (match s
                              ((Ast.List els)
                                (resume (List.len els)
                                        (Ast.List (List.push els (Ast.Int (BigInt.of v))))))
                              (_ (resume -100 s)))))
                (do
                  (def l1 (Acc.put a))
                  (def l2 (Acc.put b))
                  (+ (* 100 l1) (+ (* 10 l2) (Acc.put 3))))))
            (export main)))
  (call   main (: 1 Int64) (: 2 Int64))
  (output (: 12 Int64)))
