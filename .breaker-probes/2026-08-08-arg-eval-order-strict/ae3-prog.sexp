(do
  (effect E (op next (-> Int64)) (op mix (-> Int64 Int64 Int64 Int64)))
  (def (main (: n Int64))
    (handle E n
      ((next () s (resume s (+ s 1)))
       (mix (a b c) s (resume (+ (* 100 a) (+ (* 10 b) c)) s)))
      (E.mix (E.next) (E.next) (E.next))))
  (export main))
