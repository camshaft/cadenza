(case "ao3 control: try over Option of INT (same shape, scalar payload)"
  (input  (do
            (def (pick (: n Int64))
              (if (> n 0) (Option.Some n) (Option.None)))
            (def (grab (: n Int64))
              (do
                (def v (try (pick n)))
                (Option.Some (+ v 1))))
            (def (main (: n Int64))
              (match (grab n) ((Option.Some v) v) ((Option.None) -1)))
            (export main)))
  (call   main (: 25 Int64)) (output (: 26 Int64)))
