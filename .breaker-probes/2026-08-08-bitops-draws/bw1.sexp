(case "bw1 BIT operations over draws — mask, set-bit, and a draw-driven shift count all read the live thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 5))))
                (let ((d1 (E.next)))
                  (let ((d2 (E.next)))
                    (+ (* 100 (& d1 7))
                       (+ (* 10 (<< 1 (& d1 3)))
                          (| d2 8)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 388 Int64))
  (call   main (: 0 Int64)) (output (: 23 Int64))
  (call   main (: 6 Int64)) (output (: 651 Int64)))
