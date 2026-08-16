(case "bp3 the arm REVERSES a three-byte frame — per-index rebuild through Bytes.at, positional weights pin the swap"
  (input  (do
            (effect E (op rev (-> Bytes Bytes)))
            (def (byte-at (: b Bytes) (: i Int64))
              (match (Bytes.at b i) ((Some v) (Int64.of v)) ((None) 0)))
            (def (main (: n Int64))
              (handle E 0
                ((rev (b) s
                  (resume (Bytes.of (list (UInt8.wrap (byte-at b 2))
                                          (UInt8.wrap (byte-at b 1))
                                          (UInt8.wrap (byte-at b 0))))
                          s)))
                (let ((r (E.rev (Bytes.of (list (UInt8.wrap (if (< n 0) (- 0 n) n))
                                                (UInt8.wrap 20)
                                                (UInt8.wrap 30))))))
                  (+ (* 10000 (byte-at r 0))
                     (+ (* 100 (byte-at r 1))
                        (byte-at r 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 302005 Int64))
  (call   main (: -7 Int64)) (output (: 302007 Int64)))
