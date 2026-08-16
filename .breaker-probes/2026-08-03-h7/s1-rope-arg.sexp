(case "s1 a ROPE-built runtime String arg crosses the host boundary (H7 non-flat rep)"
  (input  (do
            (effect io (op tag (-> String Int64)))
            (def (main (: k Int64))
              (host (io)
                (io.tag (String.concat "ab" (if (> k 3) "cde" "zz")))))
            (export main)))
  (host-responses (respond io.tag (: 42 Int64)))
  (host-calls (call io.tag))
  (call   main (: 5 Int64)) (output (: 42 Int64)))
