(do (def (f (: s Symbol)) (match s (#"apple" 1) (#"banana" 2) (_ -1)))
    (def (main) (f #"cherry"))
    (export main))
