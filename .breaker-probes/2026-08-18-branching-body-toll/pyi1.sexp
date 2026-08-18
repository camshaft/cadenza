(case "pyi1 a TOLLED ARM UNDER A BRANCHING BODY — the first draw's answer picks which body branch performs the second draw so the two frames' tolls wrap DIFFERENT continuations per seed, the positive seed rides the hundred branch and the zero seed the two-hundred branch, and each frame's toll composes with whichever branch its continuation actually contains"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (* 1000 (+ s 1)))))
                (if (> (E.tick) 0)
                    (+ 100 (E.tick))
                    (+ 200 (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5102 Int64))
  (call   main (: 0 Int64)) (output (: 3201 Int64)))
