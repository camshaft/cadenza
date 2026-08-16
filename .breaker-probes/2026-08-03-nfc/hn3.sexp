(case "hn3 the NFC SEAM-composition face: a concat whose seam completes a composed char"
  (input  (do
            (def (main (: k Int64))
              (match (String.from-bytes (Bytes.of (list (UInt8.wrap 101))))
                ((Some e)
                  (match (String.from-bytes (Bytes.of (list (UInt8.wrap 204) (UInt8.wrap 129) (UInt8.wrap 122))))
                    ((Some accz)
                      (let ((joined (String.concat e accz)))
                        (+ (String.scalar-len joined)
                           (* 10 (String.byte-len joined)))))
                    ((None _u) -2)))
                ((None _u) -1)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 32 Int64)))
