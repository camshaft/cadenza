(case "nfc4 from-bytes does NOT normalize (exempt) — the decomposed bytes survive a bytes round-trip"
  (input  (do
            (def (main (: k Int64))
              (match (String.from-bytes (Bytes.of (list (UInt8.wrap 101) (UInt8.wrap 204) (UInt8.wrap 129))))
                ((Some s)
                  (+ (String.byte-len s)
                     (* 10 (Bytes.len (String.to-bytes s)))))
                ((None _u) -1)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 33 Int64)))
