(case "cv2 TWO asks each running the in-arm chain — the second chain starts where the first left the outer thread"
  (input  (do
            (effect O (op a (-> Int64)) (op b (-> Int64 Int64)))
            (effect I (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3))))
                (handle I 0
                  ((ask () t (resume (O.b (O.a)) t)))
                  (+ (* 100 (I.ask)) (I.ask)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 616 Int64))
  (call   main (: 0 Int64)) (output (: 212 Int64))
  (call   main (: -3 Int64)) (output (: -394 Int64)))
