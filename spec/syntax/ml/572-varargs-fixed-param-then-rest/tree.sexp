(def (tagged (: tag Int64) (.. (: xs (List Int64)))) (+ tag ((. List len) xs)))
