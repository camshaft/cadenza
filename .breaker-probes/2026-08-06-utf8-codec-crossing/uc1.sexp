(case "uc1 the arm DECODES a Bytes op argument to a String — multibyte UTF-8 survives the crossing"
  (input  (do
            (effect Codec (op read (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle Codec 0
                ((read (b) s
                  (resume (match (String.from-bytes b)
                            ((Some t) (String.byte-len t))
                            ((None _u) -1))
                          s)))
                (Codec.read (String.to-bytes "héllo"))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64)))
