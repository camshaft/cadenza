(case "t4 Bytes.at 0 of to-bytes of a multibyte-ending slice"
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cdé")))
                (match (String.slice s 3 5)
                  ((Some tail) (Int64.of (Option.expect (Bytes.at (String.to-bytes tail) 0) "b0")))
                  ((None u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 100 Int64)))
