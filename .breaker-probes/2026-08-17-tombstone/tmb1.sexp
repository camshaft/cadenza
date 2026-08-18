(case "tmb1 the RESUME VALUE DISCARDED in a do sequence — each arm resumes for EFFECT ONLY then answers a tombstone keyed to its own dispatch state, the body's positional fold and the inner frame's tombstone are both thrown away so only the FIRST dispatch's tombstone survives the unwind, and the seed is visible solely through that first frame's captured state"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (do (resume s (+ s 3))
                      (+ (* s 100) 7))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 107 Int64))
  (call   main (: 0 Int64)) (output (: 7 Int64)))
