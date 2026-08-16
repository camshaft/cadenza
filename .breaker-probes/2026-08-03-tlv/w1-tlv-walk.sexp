(case "w1 a recursive TLV frame walk over sliced runtime bytes decodes LE payloads per frame"
  (input  (do
            (def (walk (: b Bytes) (: off Int64) (: acc Int64))
              (match (Bytes.slice b off 3)
                ((Some frame)
                  (match frame
                    ((bin (u8 tag) (u16 val le))
                      (if (= (Int64.of tag) 0) acc
                          (walk b (+ off 3) (+ acc (Int64.of val)))))
                    (_ (- 0 acc))))
                ((None _u) acc)))
            (def (main (: k UInt8))
              (walk (Bytes.of (list (UInt8.wrap 1) k (UInt8.wrap 1)
                                    (UInt8.wrap 1) (UInt8.wrap 2) (UInt8.wrap 0)
                                    (UInt8.wrap 0) (UInt8.wrap 0) (UInt8.wrap 0))) 0 0))
            (export main)))
  (call   main (: 5 UInt8)) (output (: 263 Int64))
  (call   main (: 200 UInt8)) (output (: 458 Int64)))
