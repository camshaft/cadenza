(case "red1 reduction: init-draw only, no final tick"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (* 1000 (+ s 1)))))
                (handle E (* 10 (E.tick))
                  ((tick () s (+ (resume s (+ s 1)) (* 100 s))))
                  (E.tick))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 999999 Int64)))
