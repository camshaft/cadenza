(case "sy2 Symbol keys enumerate in canonical order and a symbol-keyed churn preserves identity"
  (input  (do
            (def syms (list #"alpha" #"beta" #"gamma" #"delta" #"epsilon"))
            (def (fill (: i Int64) (: m (Map Symbol Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (match (List.at syms (- i 1)) ((Some s) s) ((None _u) #"zz")) i))))
            (def (churn (: i Int64) (: m (Map Symbol Int64)))
              (if (= i 0) m (churn (- i 1) (Map.remove (Map.insert m #"temp" 999) #"temp"))))
            (def (main (: n Int64))
              (do
                (def direct (fill n Map.empty))
                (def churned (churn 20 (fill n Map.empty)))
                (+ (* 10 (if (= churned direct) 1 0))
                   (match (Map.lookup churned #"gamma") ((Some v) (if (= v 3) 1 0)) ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))
