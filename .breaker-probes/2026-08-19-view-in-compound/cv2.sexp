(case "cv2 a seam-crossing slice view as a SET element dedupes against its flat twin"
  (input  (do
            (def (main (: n Int64))
              (do
                (def rope (Bytes.concat (Bytes.of (list 10 20 30)) (Bytes.of (list 40 50 60 70))))
                (def w (match (Bytes.slice rope 2 3)
                         ((Some v) v) ((None _u) (Bytes.of (list)))))
                (def s (Set.of (list w (Bytes.of (list 30 40 50)) (Bytes.of (list 99)))))
                (+ (* 10 (Set.len s)) (if (Set.contains s (Bytes.of (list 30 40 50))) 1 0))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 21 Int64)))
