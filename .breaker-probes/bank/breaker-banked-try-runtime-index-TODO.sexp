(case "control2: let-bound try over runtime-index List.at"
  (input  (do
            (def (pick (: xs (List Int64)) (: k Int64))
              (let ((v (try (List.at xs k))))
                (Some (* v 10))))
            (def (main (: k Int64))
              (match (pick (list 5 6 7) k) ((Some v) v) ((None u) -1)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 60 Int64))
  (call   main (: 9 Int64)) (output (: -1 Int64)))
