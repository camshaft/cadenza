(case "pyre2 the TOLL REGION IS SEEDED FROM THE DYING FRAME'S CAPTURE — the fresh handle installed during the unwind takes its starting value from the outer frame's captured state plus twenty, the capture flows INTO the toll region while nothing flows back, and a region seeded from post-resume state instead would shift by the state advance"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (+ (resume (* s 10) (+ s 1))
                     (handle E (+ s 20)
                       ((tick () t (resume (* t 2) (+ t 1))))
                       (E.tick)))))
                (E.tick)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 52 Int64))
  (call   main (: 0 Int64)) (output (: 40 Int64)))
