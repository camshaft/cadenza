(case "do1 ONE draw consumed by BOTH branches of an if with different weights — the binder is read exactly once per run"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (+ (if (> d 2) (+ (* 100 d) 7) (- (* 10 d) 7))
                     (* 1000 (- (E.probe) n))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1507 Int64))
  (call   main (: 1 Int64)) (output (: 1003 Int64))
  (call   main (: -6 Int64)) (output (: 933 Int64)))
