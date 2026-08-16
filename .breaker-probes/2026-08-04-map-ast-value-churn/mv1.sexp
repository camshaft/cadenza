(case "mv1 Map with Ast VALUES overwritten in a loop: last-write wins per key, live tree payloads intact"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Ast)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (% i 10) (Ast.List (list (Ast.Int (BigInt.of i)) (Ast.Name "n")))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 100 (Map.len m))
                   (match (Map.lookup m 7)
                     ((Some (Ast.List els))
                       (match (List.at els 0)
                         ((Option.Some (Ast.Int b)) (Int64.of b))
                         (_ -2)))
                     (_ -1)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 1007 Int64)))
