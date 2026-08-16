(case "th1 a template HOLE performing an effect fires once at splice-evaluation, in body order"
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (pair (: chunks (List String)) (: holes (List Ast)))
              (match holes ((list a b) (quasiquote (+ (unquote a) (* 100 (unquote b))))) (_other (quote 0))))
            (def (main (: k Int64))
              (handle Ctr 1 ((tick (u) s (resume s (+ s 1))))
                (pair"{(Ctr.tick)} and {(Ctr.tick)}")))
            (export main)))
  (call   main (: 0 Int64)) (output (: 201 Int64)))
