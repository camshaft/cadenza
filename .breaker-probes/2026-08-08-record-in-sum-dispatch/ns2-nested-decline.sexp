(case "ns2 the sum-wrapped record round-trips: matched, field-updated with a draw, re-wrapped, echoed through a second op, re-matched"
  (input  (do
            (type Box (Wrap (Record (x Int64) (y Int64))))
            (effect E (op make (-> Box)) (op keep (-> Box Box)) (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((make () s (resume (Box.Wrap (record (x s) (y (+ s 2)))) (+ s 4)))
                 (keep (b) s (resume b s))
                 (next () s (resume s (+ s 4)))
                 (probe () s (resume s s)))
                (match (E.make)
                  ((Box.Wrap r)
                    (match (E.keep (Box.Wrap (Record.with r #"y" (+ (. r y) (E.next)))))
                      ((Box.Wrap r2) (+ (* 100 (. r2 x)) (+ (* 10 (. r2 y)) (- (E.probe) n)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 428 Int64))
  (call   main (: 0 Int64)) (output (: 68 Int64))
  (call   main (: -6 Int64)) (output (: -652 Int64)))
