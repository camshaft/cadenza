(do
  (effect S (op dec (-> Int64)))
  (def (main (: n Int64))
    (handle S n
      ((dec () s
        (let ((b (Bytes.of (list (UInt8.wrap s)))))
          (resume (match (String.from-bytes b)
                    ((Some t) (String.byte-len t))
                    ((None _u) -1))
                  s))))
      (S.dec)))
  (export main))
