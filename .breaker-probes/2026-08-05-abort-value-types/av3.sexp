(case "av3 the abort VALUE is a TUPLE mixing the state and a fresh heap value"
  (input  (do
            (effect St (op halt (-> Unit (Tuple (List Int64) Int64))))
            (def (main (: a Int64))
              (do
                (def t (handle St (list a (+ a 1))
                         ((halt (u) s (tuple s (* 10 (List.len s)))))
                         (St.halt)))
                (+ (List.len (. t 0)) (. t 1))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 22 Int64)))
