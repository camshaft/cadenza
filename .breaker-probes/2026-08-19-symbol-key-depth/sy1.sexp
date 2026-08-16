(case "sy1 a trie of SYMBOL keys enumerates and resolves at depth"
  (input  (do
            (def syms (list #"alpha" #"beta" #"gamma" #"delta" #"epsilon" #"zeta" #"eta" #"theta"
                            #"iota" #"kappa" #"lambda" #"mu" #"nu" #"xi" #"omicron" #"pi"
                            #"rho" #"sigma" #"tau" #"upsilon" #"phi" #"chi" #"psi" #"omega"
                            #"one" #"two" #"three" #"four" #"five" #"six" #"seven" #"eight"
                            #"nine" #"ten" #"eleven" #"twelve" #"thirteen" #"fourteen" #"fifteen" #"sixteen"))
            (def (fill (: i Int64) (: m (Map Symbol Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (match (List.at syms (- i 1)) ((Some s) s) ((None _u) #"zz")) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m #"lambda") ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 411 Int64)))
