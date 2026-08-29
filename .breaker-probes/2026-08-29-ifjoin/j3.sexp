(do (def (main (: n Int64)) (do (def b (list n 2)) (List.len (if (> n 0) (List.push b 7) (List.concat b b))))) (export main))
