(case "oar2 a HETEROGENEOUS 4-arg op (Int64/String/Bool/Int64) marshals every type to the arm"
  (input  (do
            (effect Rec (op entry (-> Int64 String Bool Int64 Int64)))
            (def (main (: n Int64))
              (handle Rec 0
                ((entry (id name flag score) s
                  (resume (+ (* 100 id) (+ (String.byte-len name) (+ (if flag 1000 0) score))) s)))
                (Rec.entry n "abc" true 7)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1510 Int64)))
