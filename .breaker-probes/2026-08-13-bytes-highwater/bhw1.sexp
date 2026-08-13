(case "bhw1 a HIGH-WATER BYTES state — byte-lexicographic max through the thread; the multibyte lead 0xC3 must rank UNSIGNED above ASCII z, and the seeded third put only wins past 0xC3"
  (input  (do
            (effect S
              (op put (-> Bytes Int64))
              (op wlen (-> Int64)))
            (def (main (: n Int64))
              (handle S (Bytes.of (list))
                ((put (bs) hw
                  (if (< hw bs)
                      (resume 1 bs)
                      (resume 0 hw)))
                 (wlen () hw (resume (Bytes.len hw) hw)))
                (let ((a (S.put (Bytes.of (list (UInt8.wrap 122))))))
                  (let ((b (S.put (Bytes.of (list (UInt8.wrap 195) (UInt8.wrap 169))))))
                    (let ((c (S.put (Bytes.of (list (UInt8.wrap n) (UInt8.wrap 200))))))
                      (let ((d (S.wlen)))
                        (+ (* 10 (+ (* 10 (+ (* 10 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 50 Int64)) (output (: 1102 Int64))
  (call   main (: 250 Int64)) (output (: 1112 Int64)))
