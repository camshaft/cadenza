(case "al2 List.update swaps one Ast node at RRB depth; neighbors and equality unaffected"
  (input  (do
            (def (build (: i Int64) (: acc (List Ast)))
              (if (= i 0) acc (build (- i 1) (List.push acc (Ast.Int (BigInt.of i))))))
            (def (main (: n Int64))
              (do
                (def xs (build n (list)))
                (def ys (List.update xs 100 (Ast.Name "swapped")))
                (+ (* 100 (match (List.at ys 100) ((Option.Some (Ast.Name _s)) 1) (_ 0)))
                   (+ (* 10 (match (List.at ys 99) ((Option.Some a) (if (= a (Ast.Int 101N)) 1 0)) (_ -1)))
                      (match (List.at xs 100) ((Option.Some a) (if (= a (Ast.Int 100N)) 1 0)) (_ -1))))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 111 Int64)))
