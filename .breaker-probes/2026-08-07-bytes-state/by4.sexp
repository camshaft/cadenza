(case "by4 nested SAME-effect handlers with independent BYTES states — the outer buffer self-doubles, the inner appends"
  (input  (do
            (effect B (op put (-> Int64)))
            (def (main (: n Int64))
              (handle B (Bytes.of (list (UInt8.wrap 1)))
                ((put () s (resume (Bytes.len s) (Bytes.concat s s))))
                (+ (B.put)
                   (+ (* 10 (handle B (Bytes.of (list (UInt8.wrap 9) (UInt8.wrap 8) (UInt8.wrap 7)))
                              ((put () t (resume (Bytes.len t) (Bytes.concat t (Bytes.of (list (UInt8.wrap 0)))))))
                              (+ (B.put) (B.put))))
                      (* 1000 (B.put))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2071 Int64)))
