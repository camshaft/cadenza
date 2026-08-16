(case "nw11 a HOST-delegated narrow op carries 999 across the component boundary in a UInt8 slot"
  (input  (do
            (effect io (op send (-> UInt8 Int64)))
            (def (main (: n Int64))
              (host (io) (io.send 999)))
            (export main)))
  (call   main (: 0 Int64))
  (host-responses (respond io.send (: 7 Int64)))
  (host-calls (call io.send (: 999 Int64)))
  (output (: 7 Int64)))
