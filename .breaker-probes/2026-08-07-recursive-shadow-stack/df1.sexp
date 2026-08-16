(case "df1 recursively STACKED same-effect handlers — the perform resolves to the DEEPEST (k=1) frame"
  (input  (do
            (effect St (op depth (-> Unit Int64)))
            (def (walk (: k Int64))
              (if (= k 0)
                  (St.depth)
                  (handle St k
                    ((depth (u) s (resume s s)))
                    (walk (- k 1)))))
            (def (main (: n Int64))
              (handle St 100
                ((depth (u) s (resume s s)))
                (walk 3)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
