(do
  (effect Give (op get (-> Unit UInt8)))
  (def (main (: n Int64))
    (handle Give 0
      ((get (u) s (resume 999 s)))
      (Int64.of (Give.get))))
  (export main))
