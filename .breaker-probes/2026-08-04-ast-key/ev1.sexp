(case "ev1 eval of a quoted expression INSIDE a handled region uses no effect context"
  (input  (do
            (effect Ask (op get (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Ask 0 ((get (_u) s (resume 7 s)))
                (+ (Ask.get) (eval (quote (+ 1 2))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 10 Int64)))
