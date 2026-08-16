(case "cc2 concat-of-concat chain, every intermediate read after (the deep kept-binding chain)"
  (input  (do
            (def (main (: k Int64))
              (let ((a (Bytes.of (list (UInt8.wrap k)))))
                (let ((ab (Bytes.concat a (Bytes.of (list (UInt8.wrap 66))))))
                  (let ((abc (Bytes.concat ab (Bytes.of (list (UInt8.wrap 67))))))
                    (+ (Bytes.len a)
                       (+ (* 10 (Bytes.len ab))
                          (+ (* 100 (Bytes.len abc))
                             (* 10000 (if (< ab abc) 1 0)))))))))
            (export main)))
  (call   main (: 65 UInt8)) (output (: 10321 Int64)))
