(case "ffs1 a FIRST-FREE-SLOT allocator over an Int64 bitmask — alloc scans for the lowest clear bit via a recursive probe answering its index, free clears a bit answering whether it was live, and the SEED pre-occupies slots so the two runs allocate around different holes"
  (input  (do
            (effect A
              (op alloc (-> Int64))
              (op freeb (-> Int64 Int64)))
            (def (lowest0 (: bits Int64) (: i Int64))
              (if (< 7 i)
                  -1
                  (if (= (& (>> bits i) 1) 0)
                      i
                      (lowest0 bits (+ i 1)))))
            (def (main (: n Int64))
              (handle A (: n Int64)
                ((alloc () bits
                  (let ((i (lowest0 bits 0)))
                    (if (< i 0)
                        (resume -1 bits)
                        (resume i (| bits (<< 1 i))))))
                 (freeb (i) bits
                  (let ((was (if (= (& bits (<< 1 i)) 0) 0 1)))
                    (resume was (& bits (^ (<< 1 i) -1))))))
                (let ((a (A.alloc)))
                  (let ((b (A.alloc)))
                    (let ((c (A.freeb 1)))
                      (let ((d (A.alloc)))
                        (let ((e (A.alloc)))
                          (let ((f (A.freeb 6)))
                            (let ((g (A.alloc)))
                              (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)) e)) f)) g))))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 20101040005 Int64))
  (call   main (: 0 Int64)) (output (: 10101020003 Int64)))
