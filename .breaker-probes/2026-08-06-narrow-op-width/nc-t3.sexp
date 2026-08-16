(do
  (effect Send (op put (-> (List UInt8) Int64)))
  (def (main (: n Int64))
    (handle Send 0
      ((put (xs) s (resume (List.len xs) s)))
      (Send.put (list 999 5))))
  (export main))
