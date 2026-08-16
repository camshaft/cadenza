(case "bf2 the same fold over the COMPACTED rope equals the rope walk (compact is value-transparent to iteration)"
  (input  (do
            (def (build (: n Int64) (: acc Bytes))
              (if (= n 0) acc (build (- n 1) (Bytes.concat acc (Bytes.of (list (UInt8.wrap n)))))))
            (def (sum (: b Bytes) (: i Int64) (: acc Int64))
              (if (>= i (Bytes.len b)) acc
                  (sum b (+ i 1) (+ acc (Int64.of (Option.expect (Bytes.at b i) "in"))))))
            (def (main (: n Int64))
              (do
                (def rope (build n (Bytes.of (list))))
                (if (= (sum rope 0 0) (sum (Bytes.compact rope) 0 0)) 1 0)))
            (export main)))
  (call   main (: 30 Int64)) (output (: 1 Int64)))
