(case "hn1 a host String RESPONSE containing decomposed text: normalized at the boundary or faithful?"
  (input  (do
            (effect io (op fetch (-> Unit String)))
            (def (main (: k Int64))
              (host (io)
                (let ((s (io.fetch unit)))
                  (+ (String.byte-len s)
                     (* 10 (if (= s "é") 1 0))))))
            (export main)))
  (host-responses (respond io.fetch (: "é" String)))
  (host-calls (call io.fetch))
  (call   main (: 0 Int64)) (output (: 12 Int64)))
