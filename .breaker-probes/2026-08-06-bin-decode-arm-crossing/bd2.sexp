(case "bd2 the ARM ENCODES its scalar op argument into framed Bytes and resumes them — body decodes"
  (input  (do
            (effect Codec (op frame (-> Int64 Bytes)))
            (def (main (: n Int64))
              (handle Codec 0
                ((frame (v) s (resume (bin (u8 (UInt8.wrap 9)) (u16 (UInt16.wrap (* v 3)))) s)))
                (match (Codec.frame (* n 10))
                  ((bin (u8 tag) (u16 val)) (+ (* 1000 tag) val))
                  (_other -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9150 Int64)))
