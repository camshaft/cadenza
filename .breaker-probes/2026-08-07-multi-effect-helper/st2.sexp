(case "st2 TWO sibling handle expressions as operands of one sum — independent regions with different arms and seeds"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (+ (handle St n
                   ((next () s (resume s (+ s 1))))
                   (+ (St.next) (St.next)))
                 (* 100 (handle St (* n 10)
                          ((next () s (resume s (- s 2))))
                          (+ (St.next) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9811 Int64))
  (call   main (: 1 Int64)) (output (: 1803 Int64)))
