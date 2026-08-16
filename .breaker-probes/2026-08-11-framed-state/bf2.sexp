(case "bf2 a MIXED-ENDIAN framed state — big-endian high word beside little-endian low word, per-segment order survives every re-frame"
  (input  (do
            (effect S (op adv (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (bin (u16 (UInt16.wrap n)) (u16 (UInt16.wrap (* n 2)) le))
                ((adv (d) st
                  (match st
                    ((bin (u16 hi) (u16 lo le))
                      (resume (+ (* 100000 hi) lo)
                              (bin (u16 (UInt16.wrap (+ hi d))) (u16 (UInt16.wrap (+ lo (* d 2))) le))))
                    (_other (resume -1 st)))))
                (let ((a (S.adv 1)))
                  (let ((b (S.adv 1)))
                    (+ a b)))))
            (export main)))
  (call   main (: 258 Int64)) (output (: 51701034 Int64))
  (call   main (: 0 Int64)) (output (: 100002 Int64)))
