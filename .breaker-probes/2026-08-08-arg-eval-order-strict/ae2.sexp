(case "ae2 draw-PURE-draw argument positions to a pure 3-ary fn — the middle constant sits between two advancing draws"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (mix3 (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (mix3 (E.next) 7 (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 576 Int64))
  (call   main (: 0 Int64)) (output (: 71 Int64)))
