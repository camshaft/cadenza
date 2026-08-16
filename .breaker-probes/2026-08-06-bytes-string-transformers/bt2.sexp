(case "bt2 a String-to-String transformer op — a rope argument crosses in, a wrapped rope crosses back"
  (input  (do
            (effect Fmt (op brack (-> String String)))
            (def (main (: n Int64))
              (handle Fmt 0
                ((brack (t) s (resume (String.concat "[" (String.concat t "]")) s)))
                (String.byte-len (Fmt.brack (String.concat "ab" (if (> n 0) "cde" "z"))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64)))
