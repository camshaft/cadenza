(case "ej6 the concrete-sibling IF control — one arm a CONCRETE list literal, the other empty"
  (input  (do
            (def (main (: n Int64))
              (let ((xs (if (> n 100) (list 1 2) (list))))
                (List.len (List.push xs n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
