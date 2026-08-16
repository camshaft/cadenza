(case "ba2 a ROPE Bytes arg (recursive concat, uncompacted) crosses the wasm boundary"
  (input  (do
            (effect io (op sink (-> Bytes Int64)))
            (def (build (: n Int64) (: acc Bytes))
              (if (> n 0) (build (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap 65))))) acc))
            (def (main (: n Int64))
              (host (io)
                (io.sink (build n (Bytes.of (list))))))
            (export main)))
  (host-responses (respond io.sink (: 42 Int64)))
  (host-calls (call io.sink))
  (call   main (: 50 Int64)) (output (: 42 Int64)))
