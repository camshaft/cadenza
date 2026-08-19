(case "hoh7 a TOLLED OUTER DRAWN AT INIT AND IN THE INNER BODY — both draws hit the ten-thousandfold-tolled outer arm so two outer frames wrap the inner machine at different points, the init draw's continuation contains the WHOLE inner handle while the body draw's contains only the region close, and the two tolls price their capture seven apart"
  (input  (do
            (effect B (op step (-> Int64)))
            (effect F (op draw (-> Int64)))
            (def (main (: n Int64))
              (handle F (% n 3)
                ((draw () st (+ (resume st (+ st 7)) (* 10000 st))))
                (handle B (F.draw)
                  ((step () s (resume (* s 10) (+ s 1))))
                  (+ (B.step) (F.draw)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 90018 Int64))
  (call   main (: 0 Int64)) (output (: 70007 Int64)))
