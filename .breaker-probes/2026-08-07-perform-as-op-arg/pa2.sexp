(case "pa2 TWO same-effect draws as the TWO arguments of one op — left-to-right arg order, the arm reads the post-args state"
  (input  (do
            (effect St (op next (-> Int64)) (op mix (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1)))
                 (mix (a b) s (resume (+ (* 100 a) (+ (* 10 b) s)) s)))
                (St.mix (St.next) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64)))
