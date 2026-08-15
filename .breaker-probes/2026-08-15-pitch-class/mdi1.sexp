(case "mdi1 a PITCH-CLASS walker over MIDI notes — transpose shifts the held note answering its class mod twelve, octave answers the note divided by twelve, two fifths then a double-octave drop then a fourth and a tone walk the circle, and the seeds' classes rotate through DIFFERENT residues while the octave rows differ by the same offset"
  (input  (do
            (effect M
              (op transpose (-> Int64 Int64))
              (op octave (-> Int64)))
            (def (main (: n Int64))
              (handle M (+ 60 n)
                ((transpose (k) note
                  (resume (% (+ note k) 12) (+ note k)))
                 (octave () note (resume (/ note 12) note)))
                (let ((a (M.transpose 7)))
                  (let ((b (M.transpose 7)))
                    (let ((c (M.octave)))
                      (let ((d (M.transpose -24)))
                        (let ((e (M.octave)))
                          (let ((f (M.transpose 5)))
                            (let ((g (M.transpose 2)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5000700050507 Int64))
  (call   main (: 0 Int64)) (output (: 7020602040709 Int64)))
