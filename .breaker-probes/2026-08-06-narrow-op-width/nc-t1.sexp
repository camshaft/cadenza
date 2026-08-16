(do
  (effect Send (op put (-> (Tuple UInt8 Int64) Int64)))
  (def (main (: n Int64))
    (handle Send 0
      ((put (p) s (match p ((tuple a b) (resume (+ (Int64.of a) b) s)))))
      (Send.put (tuple 999 5))))
  (export main))
