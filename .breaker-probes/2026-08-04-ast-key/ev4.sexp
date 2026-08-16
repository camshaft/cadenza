(case "ev4 two structurally-equal quotes built from DIFFERENT source spellings dedupe as one Set element"
  (input  (do
            (def (main (: k Int64))
              (Set.len (Set.of (list (quote (+ 1 2)) (quote (+ 1 2)) (quote (+ 1 3))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2 Int64)))
