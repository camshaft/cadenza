(case "im2min3 control: INLINE draws (no recursion) then probe"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((w (do (E.next) (E.next) (* 100 (E.next)))))
                  (+ w (- (E.probe) n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 703 Int64)))
