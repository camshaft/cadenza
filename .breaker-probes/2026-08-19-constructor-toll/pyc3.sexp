(case "pyc3 the PURE FIELD PRECEDES THE RESUME IN THE CONSTRUCTOR — the tuple lists the state capture FIRST and the resumed value second yet the answers match the reversed layout exactly, pinning that the capture is taken BY VALUE at construction from pre-resume state rather than re-read after the replay, with the compounding double and linear weights unchanged"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (match (tuple (+ s 1) (resume (* s 10) (+ s 1)))
                    ((tuple w r) (+ (* r 2) (* 1000 w))))))
                (+ (E.tick) (* 10 (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 8840 Int64))
  (call   main (: 0 Int64)) (output (: 5400 Int64)))
