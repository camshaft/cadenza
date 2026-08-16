(case "ao2 control: Ast in Option WITHOUT try (plain match on the Option)"
  (input  (do
            (def (pick (: n Int64))
              (if (> n 0) (Option.Some (Ast.Int (BigInt.of n))) (Option.None)))
            (def (main (: n Int64))
              (match (pick n)
                ((Option.Some (Ast.Int b)) (Int64.of b))
                (_ -1)))
            (export main)))
  (call   main (: 25 Int64)) (output (: 25 Int64)))
