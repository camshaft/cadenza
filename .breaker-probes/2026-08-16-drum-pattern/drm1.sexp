(case "drm1 a DRUM sequencer over sixteenth-note steps — each hit answers the firing drums as a bitmask (kick on quarters, snare on the backbeats, hat on every seed-shaped division), cnt counts non-silent steps, and the halved hat rate turns every other row SILENT while the kick and snare rows coincide exactly"
  (input  (do
            (effect D
              (op hit (-> Int64))
              (op cnt (-> Int64)))
            (def (maskof (: step Int64) (: hd Int64))
              (+ (if (= (% step 4) 0) 1 0)
                 (+ (if (= (% step 8) 4) 2 0)
                    (if (= (% step hd) 0) 4 0))))
            (def (main (: n Int64))
              (handle D (tuple (: 0 Int64) (: 0 Int64))
                ((hit () st
                  (match st
                    ((tuple step hits)
                      (match (maskof step (+ (% n 3) 1))
                        (m
                          (if (< 0 m)
                              (resume m (tuple (% (+ step 1) 16) (+ hits 1)))
                              (resume 0 (tuple (% (+ step 1) 16) hits))))))))
                 (cnt () st
                  (match st ((tuple step hits) (resume hits st)))))
                (let ((a (D.hit)))
                  (let ((b (D.hit)))
                    (let ((c (D.hit)))
                      (let ((d (D.hit)))
                        (let ((e (D.hit)))
                          (let ((f (D.hit)))
                            (let ((g (D.cnt)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5000400070003 Int64))
  (call   main (: 0 Int64)) (output (: 5040404070406 Int64)))
