(case "si1 a String op RESULT selected by the op argument, composed via concat"
  (input  (do
            (effect St (op word (-> Int64 String)))
            (def (main (: n Int64))
              (handle St 0
                ((word (k) s (resume (if (> k 0) "hi" "lo") (+ s 1))))
                (String.byte-len (String.concat (St.word n) (String.concat "-" (St.word 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
