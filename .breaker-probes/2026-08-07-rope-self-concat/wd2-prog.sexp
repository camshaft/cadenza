(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((b (bin (u8 (UInt8.wrap (St.next))))))
        (let ((dbl (Bytes.concat b b)))
          (+ (* 100 (Bytes.len dbl))
             (+ (* 10 (match (Bytes.at dbl 0) ((Some x) x) ((None _u) -1)))
                (match (Bytes.at dbl 1) ((Some y) y) ((None _u) -1))))))))
  (export main))
