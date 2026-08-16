(case "bf2 the frame grows by u16 BE records — dump decodes the SECOND record through a fixed-width first segment"
  (input  (do
            (effect W (op log (-> Int64 Int64)) (op second (-> Int64)))
            (def (main (: n Int64))
              (handle W (bin)
                ((log (v) fr (resume v (Bytes.concat fr (bin (u16 (UInt16.wrap v))))))
                 (second () fr (match fr
                                 ((bin (u16 first) (u16 mid) (bytes tl))
                                  (resume (Int64.of mid) fr))
                                 (_other (resume -1 fr)))))
                (do (W.log (+ 100 n)) (W.log (+ 200 n)) (W.log (+ 300 n)) (W.second))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 201 Int64))
  (call   main (: 4 Int64)) (output (: 204 Int64))
  (call   main (: 0 Int64)) (output (: 200 Int64)))
