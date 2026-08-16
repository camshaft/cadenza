(case "bp2 a Bytes TRANSFORMER op — the body frames a byte, the arm decodes it, adds five, re-encodes; the body decodes the transform"
  (input  (do
            (effect E (op xf (-> Bytes Bytes)))
            (def (main (: n Int64))
              (handle E 0
                ((xf (b) s
                  (match (Bytes.at b 0)
                    ((Some v) (resume (Bytes.of (list (UInt8.wrap (+ (Int64.of v) 5)))) s))
                    ((None) (resume b s)))))
                (match (Bytes.at (E.xf (Bytes.of (list (UInt8.wrap (if (< n 0) (- 0 n) n))))) 0)
                  ((Some v) (Int64.of v))
                  ((None) -9))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 15 Int64))
  (call   main (: -3 Int64)) (output (: 8 Int64)))
