(case "a from-bytes-DECODED string finds its literal twin as a Map key"
  (doc    "The DECODE-reached face of String construction-path equality at the CHAMP boundary: a
           string decoded from bytes (`String.from-bytes [104,105]` → \"hi\") must hash/compare
           content-canonically with the LITERAL \"hi\" key already in the map — lookup hits (10s
           digit) and byte-len reads 2 (1s) → 12. The BA/BB family pins concat/slice-reached string
           equality directly; the from-bytes route materializes the string through the DECODER's
           allocation path (validated fresh buffer, not a rope of existing chunks), so a hash keyed
           on allocation shape or a non-canonicalized decode output would miss the literal twin.")
  (input  (do
            (def (main (: x UInt8))
              (let ((decoded (match (String.from-bytes (Bytes.of (list 104 105)))
                               ((Some s) s) ((None u) "?"))))
                (let ((m (Map.insert Map.empty "hi" 42)))
                  (+ (* 10 (match (Map.lookup m decoded) ((Some v) 1) ((None u2) 0)))
                     (String.byte-len decoded)))))
            (export main)))
  (call   main (: 0 UInt8)) (output (: 12 Int64)))
