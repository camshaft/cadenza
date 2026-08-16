(case "aomin4 aomin3 with LET binding instead of do-def"
  (input  (do
            (def (pick (: n Int64))
              (if (> n 0) (Option.Some n) (Option.None)))
            (def (grab (: n Int64))
              (let ((a (try (pick n))))
                (Option.Some a)))
            (def (main (: n Int64))
              (match (grab n) ((Option.Some v) v) ((Option.None) -1)))
            (export main)))
  (call   main (: 25 Int64)) (output (: 25 Int64)))
