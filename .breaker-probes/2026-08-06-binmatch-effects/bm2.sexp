(case "bm2 a bin-pattern ARM performs again (parse then re-fetch on the fallback path)"
  (input  (do
            (effect St (op fetch (-> Unit Bytes)) (op log (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St n
                ((fetch (u) s (resume (bin (u8 (UInt8.wrap s))) (+ s 1)))
                 (log (v) s (resume (* v 10) s)))
                (match (St.fetch)
                  ((bin (u8 b)) (St.log (Int64.of b)))
                  (_ -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64)))
