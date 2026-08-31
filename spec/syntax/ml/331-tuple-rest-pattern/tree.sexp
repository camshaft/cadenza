(def (f t) (match t ((tuple a b (.. rest)) a) (_ 0)))
