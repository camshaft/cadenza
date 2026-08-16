(case "sc2e dissect: ONE loop-built Ast set compared to a CONSTANT small set (n=2)"
  (input  (do
            (def (up (: i Int64) (: n Int64) (: s (Set Ast)))
              (if (> i n) s (up (+ i 1) n (Set.insert s (Ast.Int (BigInt.of i))))))
            (def (main (: n Int64))
              (if (= (up 1 n (Set.of (list))) (Set.of (list (Ast.Int 1N) (Ast.Int 2N)))) 1 0))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1 Int64)))
