(case "t3 to-bytes of the WHOLE multibyte string (no slice) — control"
  (input  (do
            (def (main (: k Int64))
              (let ((s (String.concat "ab" "cdé")))
                (Bytes.len (String.to-bytes s))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 6 Int64)))
