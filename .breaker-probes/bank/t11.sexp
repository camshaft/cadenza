(case "t11 TWO Bytes.at of let-bound to-bytes of a runtime CONCAT (no slice) multibyte"
  (input  (do
            (def (main (: k Int64))
              (let ((b (String.to-bytes (String.concat "d" "é"))))
                (+ (Int64.of (Option.expect (Bytes.at b 0) "b0"))
                   (Int64.of (Option.expect (Bytes.at b 1) "b1")))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 295 Int64)))
