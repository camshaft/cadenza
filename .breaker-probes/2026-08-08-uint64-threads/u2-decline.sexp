(case "u2 the UInt64 state WRAPS at the top — three draws straddle max-1, max, zero"
  (input  (do
            (effect E (op next (-> UInt64)))
            (def (main (: u UInt64))
              (handle E (: 18446744073709551614 UInt64)
                ((next () s (resume s (UInt64.wrapping-add s (: 1 UInt64)))))
                (let ((d1 (E.next)))
                  (let ((d2 (E.next)))
                    (let ((d3 (E.next)))
                      (+ (* (: 100 UInt64) (if (= d3 (: 0 UInt64)) (: 1 UInt64) (: 5 UInt64)))
                         (+ (* (: 10 UInt64) (if (> d2 d1) (: 1 UInt64) (: 5 UInt64)))
                            (if (< d3 d1) (: 1 UInt64) (: 5 UInt64)))))))))
            (export main)))
  (call   main (: 0 UInt64)) (output (: 111 UInt64)))
