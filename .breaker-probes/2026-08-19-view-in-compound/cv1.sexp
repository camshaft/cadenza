(case "cv1 a seam-crossing slice view inside a TUPLE key matches the flat-built compound key"
  (input  (do
            (def (main (: n Int64))
              (do
                (def rope (Bytes.concat (Bytes.of (list 10 20 30)) (Bytes.of (list 40 50 60 70))))
                (def w (match (Bytes.slice rope 2 3)
                         ((Some v) v) ((None _u) (Bytes.of (list)))))
                (match (Map.lookup (Map.insert Map.empty (tuple (Bytes.of (list 30 40 50)) 1) 42)
                                   (tuple w 1))
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 42 Int64)))
