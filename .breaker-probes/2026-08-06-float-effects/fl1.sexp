(case "fl1 a Float64 handler state through a two-site arm (threshold on the magnitude)"
  (input  (do
            (effect St (op feed (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St 0.5
                ((feed (v) s (if (> v 10) (resume v (+ s 0.25)) (resume 0 s))))
                (+ (* 100 (St.feed 20)) (+ (* 10 (St.feed a)) (St.feed 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2030 Int64)))
