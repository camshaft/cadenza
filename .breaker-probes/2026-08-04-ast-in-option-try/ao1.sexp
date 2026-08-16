(case "ao1 an Ast node carried in an Option through try composes with match extraction"
  (input  (do
            (def (pick (: n Int64))
              (if (> n 0) (Option.Some (Ast.Int (BigInt.of n))) (Option.None)))
            (def (grab (: n Int64))
              (do
                (def a (try (pick n)))
                (match a ((Ast.Int b) (Option.Some (Int64.of b))) (_ (Option.None)))))
            (def (main (: n Int64))
              (match (grab n) ((Option.Some v) v) ((Option.None) -1)))
            (export main)))
  (call   main (: 25 Int64)) (output (: 25 Int64)))
