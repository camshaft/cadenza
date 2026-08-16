(case "an overflowing literal in a RECORD op argument's narrow field is rejected"
  (input  (do
            (effect Send (op put (-> (Record (small UInt8) (big Int64)) Int64)))
            (def (main (: n Int64))
              (handle Send 0
                ((put (r) s (resume (+ (Int64.of (. r small)) (. r big)) s)))
                (Send.put (record (small 999) (big 5)))))
            (export main)))
  (error  CDZ0302))
