(case "ae10 TUPLE literal of three draws matched immediately as scrutinee — positions survive the construct-then-destructure round trip"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (match (tuple (E.next) (E.next) (E.next))
                  ((tuple a b c) (+ (* 100 a) (+ (* 10 b) c))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 234 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64)))
