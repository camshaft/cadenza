(case "t12 double-read of to-bytes of a whole multibyte string bound via a helper (owned)"
  (input  (do
            (def (mk) (String.slice (String.concat "xy" "zé") 2 4))
            (def (main (: k Int64))
              (match (mk)
                ((Some v) (let ((b (String.to-bytes v)))
                            (+ (Int64.of (Option.expect (Bytes.at b 0) "0"))
                               (Int64.of (Option.expect (Bytes.at b 1) "1")))))
                ((None u) -1)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 317 Int64)))
