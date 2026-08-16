(case "ha3 INTERLEAVED outer-inner-outer draws in ONE argument list — each draw dispatches to its own handler in sequence"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op pick (-> Int64)))
            (def (mix3 (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 7
                  ((pick () s (resume s s)))
                  (mix3 (O.next) (I.pick) (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 576 Int64))
  (call   main (: 0 Int64)) (output (: 71 Int64)))
