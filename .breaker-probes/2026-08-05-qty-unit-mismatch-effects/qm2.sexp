(case "qm2 unit mismatch in the op-ARG direction rejects: op takes meter, program performs with second"
  (input  (do
            (effect St (op put (-> (Qty Int64 (Unit.base #"meter")) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((put (q) s (resume (Qty.value q) s)))
                (St.put (Qty.of n (Unit.base #"second")))))
            (export main)))
  (error  CDZ0203))
