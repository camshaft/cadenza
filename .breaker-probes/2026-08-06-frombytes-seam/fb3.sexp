(case "fb3 a TORN sequence at a rope seam (valid start byte, wrong continuation in the next leaf) is None"
  (input  (do
            (def (main (: k Int64))
              (do
                (def left (Bytes.of (list 226 152)))
                (def right (Bytes.of (list (UInt8.wrap (+ 65 k)))))
                (match (String.from-bytes (Bytes.concat left right))
                  ((Some _s) 1)
                  ((None _u) 0))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
