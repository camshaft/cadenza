(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((b (BigInt.of (St.next))))
        (let ((big (* b (* b b))))
          (Int64.of (/ big (BigInt.of 5)))))))
  (export main))
