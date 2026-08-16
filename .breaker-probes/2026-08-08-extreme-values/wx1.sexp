(case "wx1 the state thread WRAPS at Int64.max — three draws straddle the wraparound and the comparisons see the seam"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: u Int64))
              (handle E 9223372036854775806
                ((next () s (resume s (Int64.wrapping-add s 1))))
                (let ((d1 (E.next)))
                  (let ((d2 (E.next)))
                    (let ((d3 (E.next)))
                      (+ (if (> d2 d1) 100 200)
                         (+ (if (< d3 d2) 10 20)
                            (if (< d3 0) 1 2))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 111 Int64)))
