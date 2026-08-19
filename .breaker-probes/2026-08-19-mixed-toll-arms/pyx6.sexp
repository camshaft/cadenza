(case "pyx6 a TOLLED PASSIVE PEEK BETWEEN TOLLED TICKS — the peek advances nothing yet charges a hundred-thousandfold of the state it observed, the surrounding ticks charge their own thousandfold tolls, and the peek's toll must price the state BETWEEN the ticks while its passivity leaves the second tick reading exactly what the first one wrote"
  (input  (do
            (effect E (op tick (-> Int64)) (op peek (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume (* s 10) (+ s 1)) (* 1000 s)))
                 (peek () s (+ (resume s s) (* 100000 s))))
                (+ (E.tick)
                   (+ (* 10 (E.peek))
                      (* 100 (E.tick))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 205030 Int64))
  (call   main (: 0 Int64)) (output (: 102010 Int64)))
