(case "fs1 the SAME handle expression re-entered per recursion round — a FRESH region each iteration, seeds keyed by depth"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (round (: i Int64))
              (if (<= i 0)
                  0
                  (+ (handle St (* i 10)
                       ((next () s (resume s (+ s 1))))
                       (+ (St.next) (St.next)))
                     (round (- i 1)))))
            (def (main (: n Int64))
              (round n))
            (export main)))
  (call   main (: 3 Int64)) (output (: 123 Int64))
  (call   main (: 1 Int64)) (output (: 21 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
