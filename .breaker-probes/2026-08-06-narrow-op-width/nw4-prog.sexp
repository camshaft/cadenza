(do
  (effect Send (op put (-> UInt8 Int64)))
  (def (main (: n Int64))
    (handle Send 0
      ((put (v) s (resume (Int64.of v) s)))
      (Send.put 999)))
  (export main))
