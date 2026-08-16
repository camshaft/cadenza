(do
  (effect Acc (op step (-> Unit Int64)))
  (def (main (: n Int64))
    (handle Acc (: 999 UInt8)
      ((step (u) s (resume (Int64.of s) s)))
      (Acc.step)))
  (export main))
