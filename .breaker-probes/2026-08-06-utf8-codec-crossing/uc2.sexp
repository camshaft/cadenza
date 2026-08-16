(case "uc2 INVALID UTF-8 crosses as a Bytes op argument — the arm's decode declines with None"
  (input  (do
            (effect Codec (op read (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle Codec 0
                ((read (b) s
                  (resume (match (String.from-bytes b)
                            ((Some t) (String.byte-len t))
                            ((None _u) -1))
                          s)))
                (Codec.read (bin (u8 (UInt8.wrap 255)) (u8 (UInt8.wrap 254))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -1 Int64)))
