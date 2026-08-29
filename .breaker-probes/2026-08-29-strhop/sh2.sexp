(do (def (main (: n Int64)) (String.byte-len (String.concat "ab" (if (> n 0) "c" "d")))) (export main))
