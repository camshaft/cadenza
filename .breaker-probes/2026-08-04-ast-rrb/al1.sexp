(case "al1 a 200-element List of Ast nodes at RRB scale: build, index, and structurally compare"
  (input  (do
            (def (build (: i Int64) (: acc (List Ast)))
              (if (= i 0) acc (build (- i 1) (List.push acc (Ast.Int (BigInt.of i))))))
            (def (main (: n Int64))
              (do
                (def xs (build n (list)))
                (+ (* 10 (List.len xs))
                   (match (List.at xs 100)
                     ((Option.Some a) (if (= a (Ast.Int 100N)) 1 0))
                     ((Option.None) -1)))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 2001 Int64)))
