(case "me1 a MIXED-endian frame crosses the dispatch boundary as an op ARGUMENT — the arm decodes big and little fields independently"
  (input  (do
            (effect Codec (op parse (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle Codec 0
                ((parse (frame) s
                  (match frame
                    ((bin (u16 x) (u16 y le)) (resume (+ (* 100000 x) y) s))
                    (_other (resume -1 s)))))
                (Codec.parse (bin (u16 (UInt16.wrap n)) (u16 (UInt16.wrap (+ n 514)) le)))))
            (export main)))
  (call   main (: 258 Int64)) (output (: 25800772 Int64))
  (call   main (: 0 Int64)) (output (: 514 Int64)))
