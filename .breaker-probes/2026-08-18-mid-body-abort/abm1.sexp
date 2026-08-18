(case "abm1 an ABORT AFTER RESUMING DRAWS — two tick dispatches thread the state forward before a bail op answers WITHOUT resuming, the bail arm reads the state BOTH ticks built so the abort observes the aborted computation's own progress, and the body's pending fold including the thousandfold draw after the bail is abandoned wholesale"
  (input  (do
            (effect E (op tick (-> Int64)) (op bail (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (resume (+ (* s 10) 1) (+ s 1)))
                 (bail () s (+ 9000 s)))
                (+ (E.tick)
                   (+ (* 10 (E.tick))
                      (+ (E.bail) (* 1000 (E.tick)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 9003 Int64))
  (call   main (: 0 Int64)) (output (: 9002 Int64)))
