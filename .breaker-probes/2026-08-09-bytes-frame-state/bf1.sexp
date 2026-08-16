(case "bf1 a growing BYTES FRAME as handler state — each op appends a u8 record, the final op bin-decodes head + rest length"
  (input  (do
            (effect W (op log (-> Int64 Int64)) (op dump (-> Int64)))
            (def (main (: n Int64))
              (handle W (bin)
                ((log (v) fr (resume v (Bytes.concat fr (bin (u8 (UInt8.wrap v))))))
                 (dump () fr (match fr
                               ((bin (u8 hd) (bytes tl))
                                (resume (+ (* 100 (Int64.of hd)) (Bytes.len tl)) fr))
                               (_other (resume -1 fr)))))
                (do (W.log (+ 10 n)) (W.log (+ 20 n)) (W.dump))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1101 Int64))
  (call   main (: 4 Int64)) (output (: 1401 Int64))
  (call   main (: 0 Int64)) (output (: 1001 Int64)))
