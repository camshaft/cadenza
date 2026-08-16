(case "tk3e minimal: ONE inline emit + let-bound flush, no pump helper"
  (input  (do
            (effect Sink (op emit (-> Int64 Unit)) (op flush (-> Unit Bytes)))
            (def (main (: n Int64))
              (handle Sink (bin)
                ((emit (v) b (resume unit (Bytes.concat b (bin (u8 (UInt8.wrap v))))))
                 (flush (u) b (resume b b)))
                (do
                  (Sink.emit n)
                  (let ((out (Sink.flush)))
                    (Bytes.len out)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
