(case "t8 Bytes.len + one Bytes.at of a let-bound to-bytes (multibyte slice)"
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cdé")))
                (match (String.slice s 3 5)
                  ((Some tail)
                    (let ((b (String.to-bytes tail)))
                      (+ (* 100 (Bytes.len b))
                         (Int64.of (Option.expect (Bytes.at b 0) "b0")))))
                  ((None u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 400 Int64)))
