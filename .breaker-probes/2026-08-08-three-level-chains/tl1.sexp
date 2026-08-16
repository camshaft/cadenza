(case "tl1 THREE live handlers, each drawn in one expression — every draw dispatches past the two inner frames to its own"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op step (-> Int64)))
            (effect I (op pick (-> Int64)))
            (def (mix3 (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 4
                  ((step () m (resume m (+ m 2))))
                  (handle I 7
                    ((pick () t (resume t t)))
                    (+ (* 1000 (O.next)) (mix3 (M.step) (I.pick) (O.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5476 Int64))
  (call   main (: 0 Int64)) (output (: 471 Int64))
  (call   main (: -1 Int64)) (output (: -530 Int64)))
