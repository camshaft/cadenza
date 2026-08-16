(case "ab9 eval of a quoted byte-literal reconstructs the Bytes value (B2 eval-reconstruct face)"
  (input  (Bytes.len (eval (quote b"hi"))))
  (output (: 2 Int64)))
