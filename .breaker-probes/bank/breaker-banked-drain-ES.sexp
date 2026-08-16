(case "a tag-dispatched wire decode routes each frame kind through its own bin pattern"
  (doc    "The full protocol-decoder loop: TWO tag-probed bin arms with DIFFERENT frame widths (tag 1
           → 3-byte u16 frame; tag 2 → 3-byte two-u8 frame) decoding into a USER SUM, the frames
           themselves produced by bin BUILD expressions (encode→decode round-trip in one program),
           and an unknown tag falling through to the Bad variant → 12580339 (Ping 258 / Data 3,4 /
           Bad). The pinned :271 tag-then-field dispatch has ONE bin arm; multi-arm dispatch must
           probe tag 1, fail, probe tag 2 against the SAME materialized scrutinee (a consumed-offset
           leak between arm probes mis-reads the second arm's fields), and the sum round-trip pins
           the decoded payloads land in the right variant slots.")
  (input  (do
            (type Frame (Ping Int64) (Data Int64 Int64) (Bad))
            (def (decode (: b Bytes))
              (match b
                ((bin (u8 1) (u16 seq)) (Ping (Int64.of seq)))
                ((bin (u8 2) (u8 hi) (u8 lo)) (Data (Int64.of hi) (Int64.of lo)))
                (_ (Bad))))
            (def (rd (: f Frame))
              (match f
                ((Ping s) (+ 1000 s))
                ((Data h l) (+ (* h 10) l))
                ((Bad) -1)))
            (def (main (: x UInt8))
              (+ (* 10000 (rd (decode (bin (u8 1) (u16 258)))))
                 (+ (* 10 (rd (decode (bin (u8 2) (u8 x) (u8 4)))))
                    (rd (decode (bin (u8 9) (u8 0) (u8 0)))))))
            (export main)))
  (call   main (: 3 UInt8)) (output (: 12580339 Int64)))
