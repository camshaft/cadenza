(do (def (main (: n Int64))
      (do (def fs (list (fn ((: k Int64)) (+ k 1)) (fn ((: k Int64)) (* k 2))))
          (match (List.at fs (if (> n 0) 0 1)) ((Some f) (f n)) ((None _u) -1))))
    (export main))
