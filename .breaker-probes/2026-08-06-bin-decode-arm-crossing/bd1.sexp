(case "bd1 the ARM decodes a Bytes op argument with a bin pattern and resumes a parsed field"
  (input  (do
            (effect Codec (op parse (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle Codec 0
                ((parse (frame) s
                  (match frame
                    ((bin (u8 tag) (u16 val))
                      (resume (+ (* 1000 tag) val) s))
                    (_other (resume -1 s)))))
                (Codec.parse (bin (u8 (UInt8.wrap 7)) (u16 (UInt16.wrap (* n 100)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7500 Int64)))
