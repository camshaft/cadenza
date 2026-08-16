(case "fp2 FOUR bit-fields with widths 5+7+9+3 across three runtime bytes"
  (input  (do (def (run (: h Int64))
                (match (Bytes.of (list (UInt8.wrap h) (UInt8.wrap 53) (UInt8.wrap 227)))
                  ((bin (bits a 5) (bits b 7) (bits c 9) (bits d 3))
                    (+ (* 1000000 a) (+ (* 10000 b) (+ (* 10 c) d))))
                  (_other -1)))
              (export run)))
  (call   run (: 202 Int64))
  (output (: 25351883 Int64)))
