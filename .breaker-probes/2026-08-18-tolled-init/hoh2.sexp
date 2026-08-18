(case "hoh2 the INIT-COMPUTING INNER HANDLE CARRIES POST-RESUME TOLLS — the outer handler's starting value comes from an inner two-dispatch handle whose arms each add a hundredfold toll after their resume, the inner pyramid fully unwinds during INIT evaluation before the outer handler exists, and the outer draws then advance by sevens from the toll-laden seed"
  (input  (do
            (effect B (op step (-> Int64)))
            (effect F (op draw (-> Int64)))
            (def (main (: n Int64))
              (handle F (handle B (% n 3)
                          ((step () s (+ (resume (+ s 1) (* s 2)) (* 100 (+ s 1)))))
                          (+ (B.step) (* 10 (B.step))))
                ((draw () st
                  (resume st (+ st 7))))
                (let ((a (F.draw)))
                  (let ((b (F.draw)))
                    (+ a (* 1000 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 539532 Int64))
  (call   main (: 0 Int64)) (output (: 218211 Int64)))
