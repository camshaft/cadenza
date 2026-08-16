(case "im2min4 control: NON-recursive def draws then probe"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (three)
              (do (E.next) (E.next) (* 100 (E.next))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((w (three)))
                  (+ w (- (E.probe) n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 703 Int64)))
