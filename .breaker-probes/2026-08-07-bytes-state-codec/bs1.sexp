(case "bs1 a FRAMED-Bytes handler state decoded and re-encoded by the arm per dispatch"
  (input  (do
            (effect Wire (op recv (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Wire (bin (u8 (UInt8.wrap 3)) (u16 (UInt16.wrap 500)))
                ((recv (u) s
                  (match s
                    ((bin (u8 tag) (u16 val))
                      (resume (+ (* 1000 tag) val)
                              (bin (u8 (UInt8.wrap (+ tag 1))) (u16 (UInt16.wrap (+ val 10))))))
                    (_other (resume -1 s)))))
                (+ (* 10000 (Wire.recv)) (Wire.recv))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 35004510 Int64)))
