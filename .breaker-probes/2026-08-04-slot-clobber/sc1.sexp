(case "sc1 TWO marshalled args then a scalar (double-marshal slot pressure before the clobber-prone arg)"
  (input  (do
            (effect io (op send (-> String Bytes Int64 Int64)))
            (def (main (: k Int64))
              (host (io)
                (let ((n (+ k 7)))
                  (+ (io.send "tag" (Bytes.of (list 1 2)) n) n))))
            (export main)))
  (host-responses (respond io.send (: 100 Int64)))
  (host-calls (call io.send))
  (call   main (: 3 Int64))
  (output (: 110 Int64)))
