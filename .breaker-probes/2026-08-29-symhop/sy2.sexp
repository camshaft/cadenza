(do (def (main (: n Int64)) (Set.len (Set.of (list (if (> n 0) #"a" #"b") #"b" #"c")))) (export main))
