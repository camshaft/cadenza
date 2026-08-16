(case "qs1 two perform-drawn quantities of one dimension ADD — same-unit combine over two crossings"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((q1 (Qty.of (St.next) (Unit.base #"meter"))))
                  (let ((q2 (Qty.of (St.next) (Unit.base #"meter"))))
                    (Qty.value (+ q1 q2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))
