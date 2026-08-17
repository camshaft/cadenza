(case "tkb1 a TICKET BOOTH with a group comp — a group of four or more pays for one fewer than it takes (strips deducted in full, one ride comped and counted), smaller parties pay in full, either refused when the strips run short (nine-hundred row with the till), restocking adds strips, the read packs strips sold and comps, and the seed's opening till serves the mid-run trio on one booth but turns it away on the other so the restocked till and the ledgers split"
  (input  (do
            (effect T
              (op buy (-> Int64 Int64))
              (op restock (-> Int64 Int64))
              (op read (-> Int64)))
            (def (main (: n Int64))
              (handle T (tuple (+ (: 6 Int64) (* (% n 3) 4)) (: 0 Int64) (: 0 Int64))
                ((buy (k) st
                  (match st
                    ((tuple strips sold comps)
                      (if (< strips k)
                          (resume (+ (: 900 Int64) strips) st)
                          (if (>= k 4)
                              (resume (+ (* (- k 1) 10) 1)
                                      (tuple (- strips k) (+ sold (- k 1)) (+ comps 1)))
                              (resume (* k 10)
                                      (tuple (- strips k) (+ sold k) comps)))))))
                 (restock (m) st
                  (match st
                    ((tuple strips sold comps)
                      (resume (* (+ strips m) 10) (tuple (+ strips m) sold comps)))))
                 (read () st
                  (match st
                    ((tuple strips sold comps)
                      (resume (+ (* strips 100) (+ (* sold 10) comps)) st)))))
                (let ((a (T.buy (: 5 Int64))))
                  (let ((b (T.buy (: 3 Int64))))
                    (let ((c (T.restock (: 4 Int64))))
                      (let ((d (T.buy (: 4 Int64))))
                        (let ((f (T.read)))
                          (+ (* 10000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) f))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 410300600310302 Int64))
  (call   main (: 0 Int64)) (output (: 419010500310172 Int64)))
