(case "tk3g pump then a BARE post-helper dispatch in a strict operand (no let)"
  (input  (do
            (effect Sink (op emit (-> Int64 Unit)) (op flush (-> Unit Bytes)))
            (def (pump (: k Int64))
              (if (= k 0) unit
                  (do (Sink.emit 7) (pump (- k 1)))))
            (def (main (: n Int64))
              (handle Sink (bin)
                ((emit (v) b (resume unit (Bytes.concat b (bin (u8 (UInt8.wrap v))))))
                 (flush (u) b (resume b b)))
                (do
                  (pump 3)
                  (+ 100 (Bytes.len (Sink.flush))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 103 Int64)))
