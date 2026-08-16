(case "bs1 slice-of-slice-of-slice over a BYTES rope re-bases at every level"
  (input  (do
            (def (main (: k Int64))
              (do
                (def b (Bytes.concat (Bytes.of (list 10 20 30)) (Bytes.of (list (UInt8.wrap k) 50 60 70))))
                (def v1 (Option.expect (Bytes.slice b 1 5) "v1"))
                (def v2 (Option.expect (Bytes.slice v1 1 3) "v2"))
                (def v3 (Option.expect (Bytes.slice v2 1 1) "v3"))
                (+ (* 1000 (Option.expect (Bytes.at v3 0) "b0"))
                   (+ (* 10 (Bytes.len v2)) (Bytes.len v3)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 40031 Int64)))
