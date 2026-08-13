(case "bms1 a BITMASK SET in one Int64 — set ORs a shifted bit, clear ANDs with the XOR-complement answering whether the bit was live, popcount peels the mask; the clear of an absent high bit is a no-op zero"
  (input  (do
            (effect S
              (op setb (-> Int64 Int64))
              (op clearb (-> Int64 Int64))
              (op pop (-> Int64)))
            (def (count-bits (: b Int64) (: acc Int64))
              (if (= b 0) acc (count-bits (>> b 1) (+ acc (& b 1)))))
            (def (main (: n Int64))
              (handle S 0
                ((setb (i) bits
                  (let ((b2 (| bits (<< 1 i))))
                    (resume (if (= (& b2 (<< 1 i)) 0) 0 1) b2)))
                 (clearb (i) bits
                  (let ((was (if (= (& bits (<< 1 i)) 0) 0 1)))
                    (resume was (& bits (^ (<< 1 i) -1)))))
                 (pop () bits (resume (count-bits bits 0) bits)))
                (let ((a (S.setb n)))
                  (let ((b (S.setb 3)))
                    (let ((c (S.clearb n)))
                      (let ((d (S.clearb 60)))
                        (let ((e (S.pop)))
                          (+ (* 10 (+ (* 10 (+ (* 10 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 11100 Int64))
  (call   main (: 7 Int64)) (output (: 11101 Int64)))
