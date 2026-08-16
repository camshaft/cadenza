(case "pp1 a pipeline whose STAGE is branch-selected at runtime applies the taken stage"
  (input  (do
            (def (double n) (* n 2))
            (def (triple n) (* n 3))
            (def (main (: k Int64))
              (|> 5 (if (> k 0) double triple)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64)) (output (: 15 Int64)))
