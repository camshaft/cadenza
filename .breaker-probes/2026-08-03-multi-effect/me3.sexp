(case "me3 control: ONE effect with TWO ops interleaves fine"
  (input  (do
            (effect AB (op geta (-> Unit Int64)) (op getb (-> Unit Int64)))
            (def (main (: k Int64))
              (host (AB)
                (+ (AB.geta unit)
                   (+ (* 10 (AB.getb unit))
                      (* 100 (AB.geta unit))))))
            (export main)))
  (host-responses (respond a-b.geta (: 1 Int64)) (respond a-b.getb (: 2 Int64)) (respond a-b.geta (: 3 Int64)))
  (host-calls (call a-b.geta) (call a-b.getb) (call a-b.geta))
  (call   main (: 0 Int64)) (output (: 321 Int64)))
