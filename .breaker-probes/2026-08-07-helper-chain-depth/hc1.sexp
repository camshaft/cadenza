(case "hc1 a THREE-deep pure helper chain whose LEAF performs — two top-level calls thread the state through the depth"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (leaf (: k Int64)) (+ (St.next) k))
            (def (mid (: k Int64)) (* (leaf k) 2))
            (def (top (: k Int64)) (+ (mid k) 1))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (top 10) (top 100))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 244 Int64))
  (call   main (: 0 Int64)) (output (: 224 Int64)))
