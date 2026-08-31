(def (m a b) (tagged-template t (chunks "sum=" "!") (holes (+ a (* b 2)))))
