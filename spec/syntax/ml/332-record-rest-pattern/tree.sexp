(def (f r) (match r ((record (= a x) (.. rest)) x) (_ 0)))
