(case "ns1 a RECORD rides inside a sum payload through dispatch — the arm builds it from two draws, the body matches then projects"
  (input  (do
            (type Box (Wrap (Record (: x Int64) (: y Int64))))
            (effect E (op make (-> Box)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((make () s (resume (Box.Wrap (record (x s) (y (+ s 2)))) (+ s 4)))
                 (probe () s (resume s s)))
                (match (E.make)
                  ((Box.Wrap r) (+ (* 100 (. r x)) (+ (* 10 (. r y)) (- (E.probe) n)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 354 Int64))
  (call   main (: 0 Int64)) (output (: 24 Int64))
  (call   main (: -4 Int64)) (output (: -416 Int64)))
