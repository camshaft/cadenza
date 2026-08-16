(case "nfd1 NFC composes MULTI-MARK sequences in canonical order (e + cedilla-like stacking)"
  (input  (do
            (def (main (: k Int64))
              (match (String.from-bytes (Bytes.of (list (UInt8.wrap 101))))
                ((Some e)
                  (match (String.from-bytes (Bytes.of (list (UInt8.wrap 204) (UInt8.wrap 129) (UInt8.wrap 204) (UInt8.wrap 168))))
                    ((Some marks)
                      (let ((joined (String.concat e marks)))
                        (+ (String.scalar-len joined)
                           (* 10 (String.byte-len joined)))))
                    ((None _u) -2)))
                ((None _u) -1)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 42 Int64)))
