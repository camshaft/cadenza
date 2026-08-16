(case "mb2 a mid-scalar slice START is handled — slicing INTO a multibyte char declines with None"
  (input  (do
            (effect St (op cut (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((cut (b) s
                  (resume (match (Bytes.slice b 1 1)
                            ((Some w) (match (String.from-bytes w)
                                        ((Some t) (String.byte-len t))
                                        ((None _u) -7)))
                            ((None _u) -1))
                          s)))
                (St.cut (String.to-bytes "é"))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -7 Int64)))
