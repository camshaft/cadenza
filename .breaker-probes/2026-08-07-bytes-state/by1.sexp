(case "by1 a BYTES handler state grows two bytes per dispatch — each arm returns the pre-growth length"
  (input  (do
            (effect B (op put (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle B (Bytes.of (list (UInt8.wrap 65)))
                ((put (v) s (resume (Bytes.len s)
                                    (Bytes.concat s (Bytes.of (list (UInt8.wrap 66) (UInt8.wrap 67)))))))
                (+ (B.put n) (+ (* 10 (B.put n)) (* 100 (B.put n))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 531 Int64)))
