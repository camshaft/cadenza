(case "t9 TWO Bytes.at of a let-bound to-bytes of the WHOLE string (no slice) — control"
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cdé")))
                (let ((b (String.to-bytes s)))
                  (+ (Int64.of (Option.expect (Bytes.at b 0) "b0"))
                     (Int64.of (Option.expect (Bytes.at b 1) "b1"))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 197 Int64)))
