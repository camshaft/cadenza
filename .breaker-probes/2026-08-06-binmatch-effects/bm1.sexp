(case "bm1 a bin PATTERN destructures a perform-result Bytes (scrutinee from the handler)"
  (input  (do
            (effect St (op fetch (-> Unit Bytes)))
            (def (main (: n Int64))
              (handle St n
                ((fetch (u) s (resume (bin (u16 (UInt16.wrap s)) (u8 7)) (+ s 1))))
                (match (St.fetch)
                  ((bin (u16 hi) (u8 lo)) (+ (Int64.of hi) (* 100 (Int64.of lo))))
                  (_ -1))))
            (export main)))
  (call   main (: 258 Int64)) (output (: 958 Int64)))
