(case "sc3 marshalled-before-scalar with the scalar ALSO a checked-arith re-read across TWO calls"
  (input  (do
            (effect io (op a (-> String Int64 Int64)) (op b (-> Bytes Int64 Int64)))
            (def (main (: k Int64))
              (host (io)
                (let ((n (+ k 7)))
                  (+ (io.a "s" n) (+ (io.b (Bytes.of (list 9)) n) n)))))
            (export main)))
  (host-responses (respond io.a (: 10 Int64))
                  (respond io.b (: 20 Int64)))
  (host-calls (call io.a) (call io.b))
  (call   main (: 3 Int64))
  (output (: 40 Int64)))
