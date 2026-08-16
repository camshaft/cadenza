(case "sa1 STRING op arguments measured into a scalar state — the empty string is a real zero-length argument"
  (input  (do
            (effect Log (op tag (-> String Int64)))
            (def (main (: n Int64))
              (handle Log n
                ((tag (w) s (resume (+ (String.byte-len w) s) (+ s (String.byte-len w)))))
                (+ (Log.tag "ab") (+ (* 10 (Log.tag "xyz")) (* 100 (Log.tag ""))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1107 Int64))
  (call   main (: 0 Int64)) (output (: 552 Int64)))
