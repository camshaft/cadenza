(case "cf2 Bytes→String content flow under effects: perform picks the byte, decoded len observed"
  (input  (do
            (effect St (op b (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((b (u) s (resume s (+ s 1))))
                (do
                  (def raw (Bytes.of (list (UInt8.wrap (St.b)) (UInt8.wrap (St.b)))))
                  (+ (* 10 (Bytes.len raw))
                     (match (Bytes.at raw 1) ((Some v) (Int64.of v)) ((None _u) -1))))))
            (export main)))
  (call   main (: 65 Int64)) (output (: 86 Int64)))
