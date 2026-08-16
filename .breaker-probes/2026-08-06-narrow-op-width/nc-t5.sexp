(do
  (def (main (: n Int64))
    (let ((r (: (record (small 999) (big 5)) (Record (small UInt8) (big Int64)))))
      (+ (Int64.of (. r small)) (. r big))))
  (export main))
