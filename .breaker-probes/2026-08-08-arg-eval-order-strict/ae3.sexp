(case "ae3 THREE same-op draws as a 3-ary OP's own args — order pinned inside the op's argument list itself"
  (input  (do
            (effect E (op next (-> Int64)) (op mix (-> Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (mix (a b c) s (resume (+ (* 100 a) (+ (* 10 b) c)) s)))
                (E.mix (E.next) (E.next) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64)))
