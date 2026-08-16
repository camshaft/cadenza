(case "ba1 an EMPTY Bytes arg crosses the wasm host boundary"
  (input  (do
            (effect io (op sink (-> Bytes Int64)))
            (def (main (: k Int64))
              (host (io)
                (io.sink (Bytes.of (list)))))
            (export main)))
  (host-responses (respond io.sink (: 42 Int64)))
  (host-calls (call io.sink))
  (call   main (: 0 Int64)) (output (: 42 Int64)))
