(case "fb1 String.from-bytes over a ROPE whose seam splits a multibyte scalar mid-sequence"
  (input  (do
            (def (main (: k Int64))
              (do
                (def left (Bytes.of (list 195)))
                (def right (Bytes.of (list (UInt8.wrap (+ 168 k)))))
                (match (String.from-bytes (Bytes.concat left right))
                  ((Some s) (String.scalar-len s))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64)))
