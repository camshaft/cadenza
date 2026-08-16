(case "tk3 a reader→writer PUMP — Src draws pipe into Sink emits per iteration, flush reads the stream"
  (input  (do
            (effect Src (op read (-> Unit Int64)))
            (effect Sink (op emit (-> Int64 Unit)) (op flush (-> Unit Bytes)))
            (def (pump (: k Int64))
              (if (= k 0) unit
                  (do (Sink.emit (Src.read)) (pump (- k 1)))))
            (def (main (: n Int64))
              (handle Src n
                ((read (u) s (resume s (+ s 1))))
                (handle Sink (bin)
                  ((emit (v) b (resume unit (Bytes.concat b (bin (u8 (UInt8.wrap v))))))
                   (flush (u) b (resume b b)))
                  (do
                    (pump 3)
                    (let ((out (Sink.flush)))
                      (+ (* 100 (Bytes.len out))
                         (match (Bytes.at out 2) ((Some x) x) ((None _u) -1))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 307 Int64)))
