(case "aomin1 ao1 WITHOUT the Ast payload - plain Int64 in Option through try"
  (input  (do
            (def (pick (: n Int64))
              (if (> n 0) (Option.Some n) (Option.None)))
            (def (grab (: n Int64))
              (do
                (def a (try (pick n)))
                (Option.Some (* a 2))))
            (def (main (: n Int64))
              (match (grab n) ((Option.Some v) v) ((Option.None) -1)))
            (export main)))
  (call   main (: 25 Int64)) (output (: 50 Int64)))
