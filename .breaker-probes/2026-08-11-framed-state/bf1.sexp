(case "bf1 the STATE IS a framed Bytes — each dispatch bin-matches the frame, bumps the generation byte and the value word, re-frames"
  (input  (do
            (effect S (op adv (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (bin (u8 (UInt8.wrap 1)) (u16 (UInt16.wrap n)))
                ((adv (d) st
                  (match st
                    ((bin (u8 gen) (u16 val))
                      (resume (+ (* 1000 gen) val)
                              (bin (u8 (UInt8.wrap (+ gen 1))) (u16 (UInt16.wrap (+ val d))))))
                    (_other (resume -1 st)))))
                (let ((a (S.adv 10)))
                  (let ((b (S.adv 100)))
                    (+ a (* 100000 b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 201501005 Int64))
  (call   main (: 0 Int64)) (output (: 201001000 Int64)))
