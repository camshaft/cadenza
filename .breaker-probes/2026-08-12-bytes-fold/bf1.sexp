(case "bf1 a byte-wise fold over a DEEP rope (checksum walk crossing every seam)"
  (input  (do
            (def (build (: n Int64) (: acc Bytes))
              (if (= n 0) acc (build (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap n)))))))
            (def (sum (: b Bytes) (: i Int64) (: acc Int64))
              (if (>= i (Bytes.len b)) acc
                  (sum b (+ i 1) (+ acc (Int64.of (Option.expect (Bytes.at b i) "in"))))))
            (def (main (: n Int64))
              (do
                (def rope (build n (Bytes.of (list))))
                (+ (* 10 (sum rope 0 0)) (Bytes.len rope))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 4680 Int64)))
