(case "pysq2 a TOLLED REGION FOLLOWED BY AN UNTOLLED ONE — the first region's arm carries hundredfold post-resume tolls and its two frames unwind before the second region installs a plain tail-resumptive arm over the same effect, the second region's draws fold clean with no residual toll machinery, and toll infrastructure leaking across the region boundary inflates the ten-thousands"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (+ (handle E (% n 3)
                   ((tick () s (+ (resume (* s 10) (+ s 1)) (* 100 s))))
                   (+ (E.tick) (E.tick)))
                 (* 10000 (handle E (: 5 Int64)
                            ((tick () s (resume s (+ s 1))))
                            (+ (E.tick) (E.tick))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 110330 Int64))
  (call   main (: 0 Int64)) (output (: 110110 Int64)))
