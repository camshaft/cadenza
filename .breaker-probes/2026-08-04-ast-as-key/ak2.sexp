(case "ak2 a quote-built and ctor-built equal tree collide as ONE Set element at depth"
  (input  (do
            (def (fill (: i Int64) (: s (Set Ast)))
              (if (= i 0) s (fill (- i 1) (Set.insert s (Ast.Int (BigInt.of i))))))
            (def (main (: n Int64))
              (do
                (def s (Set.insert (fill n (Set.of (list))) (quote 25)))
                (+ (* 10 (Set.len s))
                   (if (Set.contains s (Ast.Int 25N)) 1 0))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 401 Int64)))
