(case "t10 TWO Bytes.at of a let-bound to-bytes of an ASCII slice — control"
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cde")))
                (match (String.slice s 3 5)
                  ((Some tail)
                    (let ((b (String.to-bytes tail)))
                      (+ (Int64.of (Option.expect (Bytes.at b 0) "b0"))
                         (Int64.of (Option.expect (Bytes.at b 1) "b1")))))
                  ((None u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 201 Int64)))
