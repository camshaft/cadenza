(case "m1 Ast.decode of RUNTIME-truncated canonical bytes is Err — the tail-cut face beside the appended-junk pin"
  (input  (do
            (def (main (: cut Int64))
              (let ((bytes (Ast.encode (quote (+ 1 (* 2 3))))))
                (match (Bytes.slice bytes 0 (- (Bytes.len bytes) cut))
                  ((Some t)
                    (match (Ast.decode t)
                      ((Ok _back) 111)
                      ((Err _e) 222)))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 222 Int64))
  (call   main (: 3 Int64)) (output (: 222 Int64)))
