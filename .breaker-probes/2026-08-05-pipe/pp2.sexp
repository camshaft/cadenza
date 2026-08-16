(case "pp2 a pipeline threading a HEAP value through closure stages preserves the source"
  (input  (do
            (def (main (: k Int64))
              (do
                (def xs (list k 2 3))
                (def grown (|> xs (List.push 9)))
                (+ (* 10 (List.len grown)) (List.len xs))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 43 Int64)))
