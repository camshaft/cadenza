(case "ae1 SUBTRACTION of two same-op draws as a 2-ary op's args — the antisymmetry pins left-to-right order exactly"
  (input  (do
            (effect E (op next (-> Int64)) (op pair (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (* s 2)))
                 (pair (a b) s (resume (- a b) s)))
                (E.pair (E.next) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -5 Int64))
  (call   main (: -3 Int64)) (output (: 3 Int64)))
