(case "ss2 a two-site arm over a BYTES state (bin-built, grown per pass)"
  (input  (do
            (effect St (op feed (-> Int64 Int64)) (op size (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (bin (u8 0))
                ((feed (v) s (if (> v 10) (resume v (Bytes.concat s (bin (u8 (UInt8.wrap v))))) (resume 0 s)))
                 (size (u) s (resume (Bytes.len s) s)))
                (+ (St.feed 20) (+ (St.feed n) (+ (St.feed 30) (* 100 (St.size)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 350 Int64)))
