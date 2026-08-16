(case "g3 a guard cond that PERFORMS advances handler state exactly once per arm try"
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k UInt8))
              (handle Ctr 0
                ((tick (u) s (resume s (+ s 1))))
                (match (Bytes.of (list (UInt8.wrap k)))
                  ((guard (bin (u8 n)) (< (+ n (Ctr.tick unit)) 10)) 111)
                  ((guard (bin (u8 m)) (< (+ m (Ctr.tick unit)) 100)) 222)
                  (_ -1))))
            (export main)))
  (call   main (: 50 UInt8)) (output (: 222 Int64))
  (call   main (: 5 UInt8)) (output (: 111 Int64)))
