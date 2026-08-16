(case "ns2 the sum-wrapped record round-trips FLATTENED — matched, field-updated with a draw, re-wrapped, echoed, re-matched at top level"
  (input  (do
            (type Box (Wrap (Record (: x Int64) (: y Int64))))
            (effect E (op make (-> Box)) (op keep (-> Box Box)) (op next (-> Int64)) (op probe (-> Int64)))
            (def (unbox (: b Box))
              (match b ((Box.Wrap r) r)))
            (def (main (: n Int64))
              (handle E n
                ((make () s (resume (Box.Wrap (record (x s) (y (+ s 2)))) (+ s 4)))
                 (keep (b) s (resume b s))
                 (next () s (resume s (+ s 4)))
                 (probe () s (resume s s)))
                (let ((r (unbox (E.make))))
                  (let ((r2 (unbox (E.keep (Box.Wrap (Record.with r #"y" (+ (. r y) (E.next))))))))
                    (+ (* 100 (. r2 x)) (+ (* 10 (. r2 y)) (- (E.probe) n)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 428 Int64))
  (call   main (: 0 Int64)) (output (: 68 Int64))
  (call   main (: -6 Int64)) (output (: -652 Int64)))
