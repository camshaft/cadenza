(case "t1 to-bytes of a slice, ASCII only (no multibyte)"
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cde")))
                (match (String.slice s 3 5)
                  ((Some tail) (Bytes.len (String.to-bytes tail)))
                  ((None u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2 Int64)))
