(case "bu2 Bytes.of with a RUNTIME Int64 (no UInt8.wrap) — does the (List (UInt 8)) sig reject it?"
  (input  (do
            (def (main (: n Int64))
              (Bytes.len (Bytes.of (list n 2 3))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 3 Int64)))
