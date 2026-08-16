(do
  (effect Send (op put (-> UInt8 Int64)))
  (def (main (: n Int64))
    (handle Send 0
      ((put (v) s (resume 7 s)))
      (Send.put 999)))
  (export main))
