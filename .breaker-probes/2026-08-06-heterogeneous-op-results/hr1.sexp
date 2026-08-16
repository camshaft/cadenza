(case "hr1 a heterogeneous TUPLE op result (String, Int64) crosses resume and destructures"
  (input  (do
            (effect Rec (op fetch (-> Int64 (Tuple String Int64))))
            (def (main (: n Int64))
              (handle Rec 0
                ((fetch (id) s (resume (tuple "row" (* id 10)) (+ s 1))))
                (match (Rec.fetch n)
                  ((tuple name score) (+ (String.byte-len name) score)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 53 Int64)))
