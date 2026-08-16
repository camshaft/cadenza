(case "fx2 fix-forward twin: composed view identity over a runtime-SELECTED rope"
  (input  (do
            (def (pick (: s Int64) (: t Bytes) (: f Bytes)) (if (= s 0) t f))
            (def (main (: s Int64))
              (do
                (def rope (Bytes.concat (pick s (Bytes.of (list 10 20 30)) (Bytes.of (list 99)))
                                        (pick s (Bytes.of (list 40 50 60 70)) (Bytes.of (list 99)))))
                (def inner (match (Bytes.slice rope 1 5)
                             ((Some outer) (match (Bytes.slice outer 1 3) ((Some i) i) ((None _u) (Bytes.of (list)))))
                             ((None _u) (Bytes.of (list)))))
                (+ (* 10 (if (= inner (Bytes.of (list 30 40 50))) 1 0))
                   (match (Map.lookup (Map.insert Map.empty (Bytes.of (list 30 40 50)) 7) inner)
                     ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 17 Int64)))
