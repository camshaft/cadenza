(case "by3 op-arg values WRAPPED to bytes at runtime accumulate in the state — the fourth emit sees three prior"
  (input  (do
            (effect B (op emit (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle B (Bytes.of (list))
                ((emit (v) s (resume (Bytes.len s)
                                     (Bytes.concat s (Bytes.of (list (UInt8.wrap v)))))))
                (do
                  (B.emit n)
                  (B.emit 77)
                  (B.emit 200)
                  (B.emit 0))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64))
  (call   main (: 255 Int64)) (output (: 3 Int64)))
