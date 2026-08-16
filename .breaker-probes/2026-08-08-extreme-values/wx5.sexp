(case "wx5 the state STEPS by wrapping-sub of MAX each draw — two hops cross the seam in opposite directions"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: u Int64))
              (handle E 0
                ((next () s (resume s (Int64.wrapping-sub s 9223372036854775807))))
                (let ((d1 (E.next)))
                  (let ((d2 (E.next)))
                    (let ((d3 (E.next)))
                      (+ (if (< d2 0) 1 5)
                         (+ (if (> d3 0) 10 50)
                            (if (= d3 2) 100 900))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 111 Int64)))
