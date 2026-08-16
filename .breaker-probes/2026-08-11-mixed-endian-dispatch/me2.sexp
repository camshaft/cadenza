(case "me2 the arm RE-ENCODES its two decoded fields with SWAPPED endianness and the body decodes the swap"
  (input  (do
            (effect Codec (op flip (-> Bytes Bytes)))
            (def (main (: n Int64))
              (handle Codec 0
                ((flip (frame) s
                  (match frame
                    ((bin (u16 x) (u16 y le))
                      (resume (bin (u16 (UInt16.wrap x) le) (u16 (UInt16.wrap y))) s))
                    (_other (resume frame s)))))
                (match (Codec.flip (bin (u16 (UInt16.wrap n)) (u16 (UInt16.wrap (+ n 3)) le)))
                  ((bin (u16 a le) (u16 b)) (+ (* 100000 a) b))
                  (_other -1))))
            (export main)))
  (call   main (: 258 Int64)) (output (: 25800261 Int64))
  (call   main (: 500 Int64)) (output (: 50000503 Int64)))
