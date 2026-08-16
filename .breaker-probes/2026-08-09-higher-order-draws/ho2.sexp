(case "ho2 a PURE factory returns a closure applied to a draw — the captured constant is pure, only the argument is drawn"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (mk (: k Int64)) (fn ((: x Int64)) (+ (* x k) 1)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((f (mk 7)))
                  (+ (* 10 (f (E.next))) (- (E.probe) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 221 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -2 Int64)) (output (: -129 Int64)))
