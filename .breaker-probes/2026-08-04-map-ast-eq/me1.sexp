(case "me1 = over two loop-built Maps with Ast KEYS (boundary companion of the Set Ast eq decline)"
  (input  (do
            (def (up (: i Int64) (: n Int64) (: m (Map Ast Int64)))
              (if (> i n) m (up (+ i 1) n (Map.insert m (Ast.Int (BigInt.of i)) i))))
            (def (main (: n Int64))
              (if (= (up 1 n Map.empty) (up 1 n Map.empty)) 1 0))
            (export main)))
  (call   main (: 50 Int64)) (output (: 1 Int64)))
