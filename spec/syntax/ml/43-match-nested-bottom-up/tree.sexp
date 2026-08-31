(def (f p) (match p ((tuple a b) (match b ((tuple c d) (+ (+ a c) d))))))
