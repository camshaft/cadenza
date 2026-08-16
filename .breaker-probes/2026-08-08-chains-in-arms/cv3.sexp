(case "cv3 the in-arm chain routes through a PURE fn mid-hop — O.b of dbl of O.a, the pure call must not detach the thread"
  (input  (do
            (effect O (op a (-> Int64)) (op b (-> Int64 Int64)) (op probe (-> Int64)))
            (effect I (op ask (-> Int64)))
            (def (dbl (: x Int64)) (* 2 x))
            (def (main (: n Int64))
              (handle O n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (probe () s (resume s s)))
                (handle I 0
                  ((ask () t (resume (O.b (dbl (O.a))) t)))
                  (+ (* 10 (I.ask)) (O.probe)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 87 Int64))
  (call   main (: 0 Int64)) (output (: 25 Int64))
  (call   main (: -4 Int64)) (output (: -99 Int64)))
