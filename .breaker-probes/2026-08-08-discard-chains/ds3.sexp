(case "ds3 a DISCARDED handle expression whose interior draws from the outer thread — the frame opens, draws, closes, and the value dies"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect I (op pick (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (do (handle I 0
                      ((pick () t (resume t t)))
                      (do (E.next) (I.pick)))
                    (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -7 Int64)) (output (: -6 Int64)))
