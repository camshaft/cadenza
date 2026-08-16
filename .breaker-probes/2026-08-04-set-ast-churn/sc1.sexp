(case "sc1 a Set of Ast nodes churned to HALF: insert 2n, remove odds, membership by structural eq"
  (input  (do
            (def (ins (: i Int64) (: s (Set Ast)))
              (if (= i 0) s (ins (- i 1) (Set.insert s (Ast.Int (BigInt.of i))))))
            (def (rem (: i Int64) (: s (Set Ast)))
              (if (> i 200) s (rem (+ i 2) (Set.remove s (Ast.Int (BigInt.of i))))))
            (def (main (: n Int64))
              (do
                (def s (rem 1 (ins n (Set.of (list)))))
                (+ (* 10 (Set.len s))
                   (+ (if (Set.contains s (Ast.Int 100N)) 1 0)
                      (if (Set.contains s (Ast.Int 99N)) 100 0)))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 1001 Int64)))
