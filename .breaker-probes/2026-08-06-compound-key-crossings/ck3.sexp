(case "ck3 a STRING-keyed map whose key is built from a PERFORM RESULT — effect-derived compound lookup"
  (input  (do
            (effect St (op tag (-> Int64 String)) (op fetch (-> String Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((tag (k) s (resume (if (> k 0) "hot" "cold") (+ s 1)))
                 (fetch (name) s (resume (+ (String.byte-len name) (* s 10)) (+ s 1))))
                (St.fetch (String.concat (St.tag n) "-path"))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 18 Int64)))
