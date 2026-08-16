(case "qs3 a perform-drawn quantity DIVIDES across dimensions — the meter/second quotient value"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((d (Qty.of (* (St.next) 6) (Unit.base #"meter"))))
                  (let ((t (Qty.of 3 (Unit.base #"second"))))
                    (Qty.value (/ d t))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64)))
