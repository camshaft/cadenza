(do (def (main (: n Int64)) (do (def b (list n 2)) (List.len (if (> n 0) (List.concat b (list 3)) (List.concat (list 0) b))))) (export main))
