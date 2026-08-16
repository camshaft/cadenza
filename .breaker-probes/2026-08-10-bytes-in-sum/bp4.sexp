(case "bp4 two one-byte op results CONCATENATED in the body — the joined frame's length and both bytes decode"
  (input  (do
            (effect E (op mk (-> Int64 Bytes)))
            (def (byte-at (: b Bytes) (: i Int64))
              (match (Bytes.at b i) ((Some v) (Int64.of v)) ((None) 0)))
            (def (main (: n Int64))
              (handle E 0
                ((mk (v) s (resume (Bytes.of (list (UInt8.wrap v))) s)))
                (let ((j (Bytes.concat (E.mk (if (< n 0) (- 0 n) n)) (E.mk 42))))
                  (+ (* 1000 (Bytes.len j))
                     (+ (* 10 (byte-at j 0))
                        (- (byte-at j 1) 40))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2052 Int64))
  (call   main (: -9 Int64)) (output (: 2092 Int64)))
