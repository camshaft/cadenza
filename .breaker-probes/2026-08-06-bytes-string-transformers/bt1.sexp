(case "bt1 a Bytes-to-Bytes transformer op — the arm frames the payload it received and the body re-reads"
  (input  (do
            (effect Codec (op wrap (-> Bytes Bytes)))
            (def (main (: n Int64))
              (handle Codec 0
                ((wrap (b) s (resume (Bytes.concat (bin (u8 (UInt8.wrap (Bytes.len b)))) b) s)))
                (let ((out (Codec.wrap (bin (u8 (UInt8.wrap (* n 8))) (u8 (UInt8.wrap 3))))))
                  (+ (* 10000 (Bytes.len out))
                     (+ (* 100 (match (Bytes.at out 0) ((Some h) h) ((None _u) -1)))
                        (match (Bytes.at out 1) ((Some p) p) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30240 Int64)))
