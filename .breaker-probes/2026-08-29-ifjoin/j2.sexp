(do (def (main (: n Int64)) (do (def b (list n 2 9)) (List.len (if (> n 0) (List.push b 7) (List.update b 1 5))))) (export main))
