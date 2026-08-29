(do (def (main (: n Int64)) (Char.to-int (String.at (if (> n 0) "abc" "xyz") 1))) (export main))
