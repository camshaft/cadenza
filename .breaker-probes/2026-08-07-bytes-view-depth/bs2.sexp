(case "bs2 a slice VIEW straddling the rope seam, concatenated with another view, equals the direct build"
  (input  (do
            (def (main (: k Int64))
              (do
                (def b (Bytes.concat (Bytes.of (list 1 2 3)) (Bytes.of (list (UInt8.wrap k) 5))))
                (def left (Option.expect (Bytes.slice b 2 2) "l"))
                (def right (Option.expect (Bytes.slice b 0 1) "r"))
                (+ (* 10 (if (= (Bytes.concat left right) (Bytes.of (list 3 (UInt8.wrap k) 1))) 1 0))
                   (if (= (Bytes.concat left right) (Bytes.of (list 3 4 2))) 1 0))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 10 Int64)))
