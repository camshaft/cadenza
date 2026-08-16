(case "at4 an ABORTING arm (no resume) discards the continuation and answers with a value derived from an Ast op-arg"
  (input  (do
            (effect Halt (op stop (-> Ast Int64)))
            (def (main (: n Int64))
              (+ 1000
                 (handle Halt 0
                   ((stop (a) s (match a ((Ast.Int b) (+ (Int64.of b) s)) (_ -1))))
                   (+ 500 (Halt.stop (Ast.Int (BigInt.of n)))))))
            (export main)))
  (call   main (: 25 Int64)) (output (: 1025 Int64)))
