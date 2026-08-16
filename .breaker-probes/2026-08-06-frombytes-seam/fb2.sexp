(case "fb2 from-bytes over a 3-way rope splitting a 3-byte scalar at BOTH seams"
  (input  (do
            (def (main (: k Int64))
              (do
                (def a (Bytes.of (list (UInt8.wrap (+ 225 k)))))
                (def b (Bytes.of (list 152)))
                (def c (Bytes.of (list 143)))
                (match (String.from-bytes (Bytes.concat (Bytes.concat a b) c))
                  ((Some s) (+ (* 10 (String.scalar-len s)) (String.byte-len s)))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 13 Int64)))
