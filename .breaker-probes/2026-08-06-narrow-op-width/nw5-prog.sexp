(do
  (def (f (: v UInt8)) (Int64.of v))
  (def (main (: n Int64)) (f 999))
  (export main))
