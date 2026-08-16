(case "sc2 two Ast Sets built in OPPOSITE orders are equal (order-independent convergence over node hashing)"
  (input  (do
            (def (up (: i Int64) (: n Int64) (: s (Set Ast)))
              (if (> i n) s (up (+ i 1) n (Set.insert s (Ast.Int (BigInt.of i))))))
            (def (down (: i Int64) (: s (Set Ast)))
              (if (= i 0) s (down (- i 1) (Set.insert s (Ast.Int (BigInt.of i))))))
            (def (main (: n Int64))
              (if (= (up 1 n (Set.of (list))) (down n (Set.of (list)))) 1 0))
            (export main)))
  (call   main (: 100 Int64)) (output (: 1 Int64)))
