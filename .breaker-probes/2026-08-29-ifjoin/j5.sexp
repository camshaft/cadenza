(do (def (main (: n Int64)) (do (def s (String.concat "ab" (if (> n 5) "c" "d"))) (String.byte-len (if (> n 0) (String.concat s "x") (String.concat "y" s))))) (export main))
