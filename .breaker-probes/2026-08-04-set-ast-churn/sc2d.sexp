(case "sc2d dissect: = over two LOOP-BUILT Ast sets, SAME order both (one build fn)"
  (input  (do
            (def (up (: i Int64) (: n Int64) (: s (Set Ast)))
              (if (> i n) s (up (+ i 1) n (Set.insert s (Ast.Int (BigInt.of i))))))
            (def (main (: n Int64))
              (if (= (up 1 n (Set.of (list))) (up 1 n (Set.of (list)))) 1 0))
            (export main)))
  (call   main (: 100 Int64)) (output (: 1 Int64)))
