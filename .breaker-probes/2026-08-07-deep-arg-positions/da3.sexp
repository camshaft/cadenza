(case "da3 draws as SET-literal elements — n=7 collides the first draw with the fixed element, n=6 collides the second"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (Set.len (Set.of (list 7 (St.next) (St.next))))
                   (* 100 (St.next)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 902 Int64))
  (call   main (: 6 Int64)) (output (: 802 Int64))
  (call   main (: 0 Int64)) (output (: 203 Int64)))
