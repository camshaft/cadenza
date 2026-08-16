(case "sg7 byte-len vs scalar-len DIVERGE on a growing multi-byte rope — each dispatch reads the difference (one per accent)"
  (input  (do
            (effect St (op grow (-> Int64)))
            (def (main (: n Int64))
              (handle St "é"
                ((grow () s (resume (- (String.byte-len s) (String.scalar-len s))
                                    (String.concat s "é"))))
                (+ (St.grow) (+ (* 10 (St.grow)) (* 100 (St.grow))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 321 Int64)))
