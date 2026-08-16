(case "tk2 an emit/flush byte-writer seeded EMPTY — three emits accumulate, flush reads the frame back"
  (input  (do
            (effect Sink (op emit (-> Int64 Unit)) (op flush (-> Unit Bytes)))
            (def (main (: n Int64))
              (handle Sink (bin)
                ((emit (v) b (resume unit (Bytes.concat b (bin (u8 (UInt8.wrap v))))))
                 (flush (u) b (resume b b)))
                (do
                  (Sink.emit n)
                  (Sink.emit 9)
                  (Sink.emit 2)
                  (let ((out (Sink.flush)))
                    (+ (* 100 (Bytes.len out))
                       (match (Bytes.at out 1) ((Some x) x) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 309 Int64)))
