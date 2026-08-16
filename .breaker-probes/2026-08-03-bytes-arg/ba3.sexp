(case "ba3 a Bytes arg used TWICE: sent to the host AND re-read after (consuming-op discipline at the arg site)"
  (input  (do
            (effect io (op sink (-> Bytes Int64)))
            (def (main (: k Int64))
              (host (io)
                (let ((b (String.to-bytes (String.concat "ab" (if (> k 100) "z" "cde")))))
                  (+ (io.sink b)
                     (* 10 (Bytes.len b))))))
            (export main)))
  (host-responses (respond io.sink (: 7 Int64)))
  (host-calls (call io.sink))
  (call   main (: 0 Int64)) (output (: 57 Int64)))
