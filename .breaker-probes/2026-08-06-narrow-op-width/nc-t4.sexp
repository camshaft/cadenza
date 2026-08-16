(do
  (def (f (: r (Record (small UInt8) (big Int64)))) (+ (Int64.of (. r small)) (. r big)))
  (def (main (: n Int64)) (f (record (small 999) (big 5))))
  (export main))
