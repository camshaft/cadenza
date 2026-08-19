(case "pyre1 a WHOLE HANDLE EXPRESSION AS THE TOLL — the post-resume toll is itself a fresh region over the SAME effect installed during the unwind, the toll-region's arm answers a hundred-shifted draw from its own seed of seven, and the unwinding frame's fresh region neither sees the outer state nor leaks its own"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (+ (resume (* s 10) (+ s 1))
                     (handle E (: 7 Int64)
                       ((tick () t (resume (+ t 100) (+ t 1))))
                       (E.tick)))))
                (E.tick)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 117 Int64))
  (call   main (: 0 Int64)) (output (: 107 Int64)))
