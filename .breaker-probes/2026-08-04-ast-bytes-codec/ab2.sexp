(case "ab2 Ast.decode of a TRUNCATED bytes encoding is Err, not a trap or a short read"
  (input  (do
            (def (main)
              (do
                (def enc (Ast.encode (Ast.Bytes b"hi")))
                (def cut (Option.expect (Bytes.slice enc 0 (- (Bytes.len enc) 1)) "in bounds"))
                (match (Ast.decode cut) ((Ok _a) -1) ((Err _e) 7))))
            (export main)))
  (output (: 7 Int64)))
