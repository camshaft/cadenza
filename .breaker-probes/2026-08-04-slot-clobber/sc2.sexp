(case "sc2 scalar BETWEEN two marshalled args (marshal-scalar-marshal sandwich)"
  (input  (do
            (effect io (op send (-> String Int64 Bytes Int64)))
            (def (main (: k Int64))
              (host (io)
                (let ((n (* k 3)))
                  (+ (io.send "x" n (Bytes.of (list (UInt8.wrap k)))) n))))
            (export main)))
  (host-responses (respond io.send (: 50 Int64)))
  (host-calls (call io.send))
  (call   main (: 4 Int64))
  (output (: 62 Int64)))
