(case "t2 to-bytes of a slice ending at a multibyte scalar"
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cdé")))
                (match (String.slice s 3 5)
                  ((Some tail) (Bytes.len (String.to-bytes tail)))
                  ((None u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 3 Int64)))
