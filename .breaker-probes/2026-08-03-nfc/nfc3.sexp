(case "nfc3 normalized runtime strings dedupe as CHAMP keys + intern to equal Symbols"
  (input  (do
            (def (main (: k Int64))
              (match (String.from-bytes (Bytes.of (list (UInt8.wrap 101))))
                ((Some e)
                  (match (String.from-bytes (Bytes.of (list (UInt8.wrap 204) (UInt8.wrap 129))))
                    ((Some acc)
                      (let ((decomposed (String.concat e acc)))
                        (+ (Set.len (Set.of (list decomposed "é")))
                           (* 10 (if (= (Symbol.of decomposed) (Symbol.of "é")) 1 0)))))
                    ((None _u) -2)))
                ((None _u) -1)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
