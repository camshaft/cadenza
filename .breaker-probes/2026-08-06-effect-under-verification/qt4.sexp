(case "qt4 a MAP handler state through a two-site arm + body free-var (does the ts orphan hit Map?)"
  (input  (do
            (effect Acc (op feed (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle Acc Map.empty
                ((feed (v) s (if (> v 10) (resume v (Map.insert s v v)) (resume 0 s))))
                (+ a (+ (Acc.feed 20) (+ (Acc.feed 3) (Acc.feed 30))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 55 Int64)))
