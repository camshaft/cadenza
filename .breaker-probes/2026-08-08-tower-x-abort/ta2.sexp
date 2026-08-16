(case "ta2 an abort INSIDE the inner frame of a same-effect tower — Bail innermost, both E threads keep their positions"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (inner-run (: k Int64))
              (handle E (* 10 k)
                ((next () s (resume s (+ s 5))))
                (+ (handle Bail 0
                     ((out (v) t (+ 1000 v)))
                     (let ((d (E.next)))
                       (if (> d 52) (Bail.out d) d)))
                   (E.next))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 10 (inner-run (E.next))) (- (E.next) n))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 451 Int64))
  (call   main (: 6 Int64)) (output (: 11251 Int64))
  (call   main (: 0 Int64)) (output (: 51 Int64))
  (call   main (: -3 Int64)) (output (: -549 Int64)))
