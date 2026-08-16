(case "t6 bind to-bytes to a name, then Bytes.at (multibyte slice)"
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cdé")))
                (match (String.slice s 3 5)
                  ((Some tail)
                    (let ((b (String.to-bytes tail)))
                      (Int64.of (Option.expect (Bytes.at b 0) "b0"))))
                  ((None u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 100 Int64)))
