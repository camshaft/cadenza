(case "me2 = over two loop-built Maps with Ast VALUES (Int keys)"
  (input  (do
            (def (up (: i Int64) (: n Int64) (: m (Map Int64 Ast)))
              (if (> i n) m (up (+ i 1) n (Map.insert m i (Ast.Int (BigInt.of i))))))
            (def (main (: n Int64))
              (if (= (up 1 n Map.empty) (up 1 n Map.empty)) 1 0))
            (export main)))
  (call   main (: 50 Int64)) (output (: 1 Int64)))
