(case "da2 draws as MAP-literal keys and values — two entries built from three sequential draws"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (Map.len (map ((St.next) (St.next)) ((St.next) 100)))
                   (* 10 (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 82 Int64))
  (call   main (: 0 Int64)) (output (: 32 Int64)))
